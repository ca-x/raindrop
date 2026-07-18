use sea_orm::{
    ConnectionTrait, DatabaseBackend, DbBackend, DbErr, QueryResult, RuntimeErr, SqlxError,
    SqlxMySqlError, SqlxPostgresError, SqlxSqliteError, Statement,
};
use serde::Serialize;
use uuid::Uuid;

use super::{RefreshClaim, RefreshCounts, RefreshRepositoryError, RefreshStatus};

const PAYLOAD_VERSION: i32 = 1;
const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
const FEED_AGGREGATE_TYPE: &str = "FEED";
const REFRESH_PERSISTED_EVENT_TYPE: &str = "feed.refresh.persisted";
const REFRESH_COMPLETED_EVENT_TYPE: &str = "feed.refresh.completed";
const PERSISTED_EVENT_SEQUENCE: i32 = 10;
const COMPLETED_EVENT_SEQUENCE: i32 = 20;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedPayload<'a> {
    event_type: &'static str,
    payload_version: i32,
    refresh_id: &'a str,
    feed_id: &'a str,
    commit_generation: Option<i64>,
    new_count: i32,
    updated_count: i32,
    dropped_count: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompletedPayload<'a> {
    event_type: &'static str,
    payload_version: i32,
    refresh_id: &'a str,
    feed_id: &'a str,
    status: &'static str,
    http_status: Option<i32>,
    new_count: i32,
    updated_count: i32,
    dropped_count: i32,
    error_code: Option<&'a str>,
}

struct LifecycleEvent<'a> {
    event_type: &'static str,
    aggregate_id: &'a str,
    refresh_id: &'a str,
    event_sequence: i32,
    payload_json: String,
    idempotency_key: String,
}

struct ExistingLifecycleEvent {
    event_type: String,
    aggregate_type: String,
    aggregate_id: String,
    refresh_id: String,
    event_sequence: i32,
    payload_version: i32,
    payload_json: String,
    idempotency_key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UniqueViolationProvenance<'a> {
    Postgres(&'a str),
    MySql(u16),
    Sqlite(i32),
    Other,
}

pub(super) fn valid_error_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

pub(super) async fn record_persisted_event<C>(
    connection: &C,
    backend: DbBackend,
    claim: &RefreshClaim,
    counts: RefreshCounts,
    generation: Option<i64>,
) -> Result<(), RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let event = persisted_event(claim, counts, generation)?;
    record_event(connection, backend, &event).await
}

pub(super) async fn record_completed_event<C>(
    connection: &C,
    backend: DbBackend,
    claim: &RefreshClaim,
    status: RefreshStatus,
    http_status: Option<i32>,
    counts: RefreshCounts,
    error_code: Option<&str>,
) -> Result<(), RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let event = completed_event(claim, status, http_status, counts, error_code)?;
    record_event(connection, backend, &event).await
}

fn persisted_event<'a>(
    claim: &'a RefreshClaim,
    counts: RefreshCounts,
    generation: Option<i64>,
) -> Result<LifecycleEvent<'a>, RefreshRepositoryError> {
    validate_identity(claim)?;
    validate_counts(counts)?;
    let payload_json = encode_payload(&PersistedPayload {
        event_type: REFRESH_PERSISTED_EVENT_TYPE,
        payload_version: PAYLOAD_VERSION,
        refresh_id: &claim.run_id,
        feed_id: &claim.feed_id,
        commit_generation: generation,
        new_count: counts.new_count,
        updated_count: counts.updated_count,
        dropped_count: counts.dropped_count,
    })?;
    Ok(LifecycleEvent {
        event_type: REFRESH_PERSISTED_EVENT_TYPE,
        aggregate_id: &claim.feed_id,
        refresh_id: &claim.run_id,
        event_sequence: PERSISTED_EVENT_SEQUENCE,
        payload_json,
        idempotency_key: format!("refresh:{}:persisted:v1", claim.run_id),
    })
}

fn completed_event<'a>(
    claim: &'a RefreshClaim,
    status: RefreshStatus,
    http_status: Option<i32>,
    counts: RefreshCounts,
    error_code: Option<&'a str>,
) -> Result<LifecycleEvent<'a>, RefreshRepositoryError> {
    validate_identity(claim)?;
    validate_counts(counts)?;
    if error_code.is_some_and(|value| !valid_error_code(value)) {
        return Err(RefreshRepositoryError::InvalidRequest);
    }
    if matches!(
        status,
        RefreshStatus::Queued
            | RefreshStatus::Running
            | RefreshStatus::LeaseLost
            | RefreshStatus::Cancelled
    ) {
        return Err(RefreshRepositoryError::InvalidRequest);
    }
    let payload_json = encode_payload(&CompletedPayload {
        event_type: REFRESH_COMPLETED_EVENT_TYPE,
        payload_version: PAYLOAD_VERSION,
        refresh_id: &claim.run_id,
        feed_id: &claim.feed_id,
        status: status.as_str(),
        http_status,
        new_count: counts.new_count,
        updated_count: counts.updated_count,
        dropped_count: counts.dropped_count,
        error_code,
    })?;
    Ok(LifecycleEvent {
        event_type: REFRESH_COMPLETED_EVENT_TYPE,
        aggregate_id: &claim.feed_id,
        refresh_id: &claim.run_id,
        event_sequence: COMPLETED_EVENT_SEQUENCE,
        payload_json,
        idempotency_key: format!("refresh:{}:completed:v1", claim.run_id),
    })
}

fn validate_identity(claim: &RefreshClaim) -> Result<(), RefreshRepositoryError> {
    if claim.run_id.is_empty()
        || claim.run_id.len() > 36
        || claim.feed_id.is_empty()
        || claim.feed_id.len() > 36
    {
        Err(RefreshRepositoryError::InvalidRequest)
    } else {
        Ok(())
    }
}

fn validate_counts(counts: RefreshCounts) -> Result<(), RefreshRepositoryError> {
    if counts.new_count < 0 || counts.updated_count < 0 || counts.dropped_count < 0 {
        Err(RefreshRepositoryError::InvalidRequest)
    } else {
        Ok(())
    }
}

fn encode_payload<T: Serialize>(payload: &T) -> Result<String, RefreshRepositoryError> {
    let payload = serde_json::to_string(payload)
        .map_err(|_| RefreshRepositoryError::InvalidLifecyclePayload)?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(RefreshRepositoryError::LifecyclePayloadTooLarge);
    }
    Ok(payload)
}

async fn record_event<C>(
    connection: &C,
    backend: DbBackend,
    event: &LifecycleEvent<'_>,
) -> Result<(), RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    if let Some(existing) = find_by_idempotency(connection, backend, &event.idempotency_key).await?
    {
        return ensure_same_event(&existing, event);
    }
    if find_by_order(connection, backend, event.refresh_id, event.event_sequence)
        .await?
        .is_some()
    {
        return Err(RefreshRepositoryError::LifecycleEventConflict);
    }

    match connection
        .execute(insert_event_statement(backend, event))
        .await
    {
        Ok(result) if result.rows_affected() == 1 => Ok(()),
        Ok(_) => Err(RefreshRepositoryError::CorruptData),
        Err(error) if is_unique_violation(&error) => {
            Err(RefreshRepositoryError::LifecycleEventConflict)
        }
        Err(error) => Err(RefreshRepositoryError::Database(error)),
    }
}

fn ensure_same_event(
    existing: &ExistingLifecycleEvent,
    event: &LifecycleEvent<'_>,
) -> Result<(), RefreshRepositoryError> {
    if existing.event_type == event.event_type
        && existing.aggregate_type == FEED_AGGREGATE_TYPE
        && existing.aggregate_id == event.aggregate_id
        && existing.refresh_id == event.refresh_id
        && existing.event_sequence == event.event_sequence
        && existing.payload_version == PAYLOAD_VERSION
        && existing.payload_json == event.payload_json
        && existing.idempotency_key == event.idempotency_key
    {
        Ok(())
    } else {
        Err(RefreshRepositoryError::LifecycleEventConflict)
    }
}

async fn find_by_idempotency<C>(
    connection: &C,
    backend: DbBackend,
    idempotency_key: &str,
) -> Result<Option<ExistingLifecycleEvent>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Postgres => {
            "SELECT event_type, aggregate_type, aggregate_id, refresh_id, event_sequence,
                    payload_version, payload_json, idempotency_key
             FROM lifecycle_outbox WHERE idempotency_key=$1"
        }
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT event_type, aggregate_type, aggregate_id, refresh_id, event_sequence,
                    payload_version, payload_json, idempotency_key
             FROM lifecycle_outbox WHERE idempotency_key=?"
        }
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [idempotency_key.into()],
        ))
        .await?
        .map(decode_existing_event)
        .transpose()
}

async fn find_by_order<C>(
    connection: &C,
    backend: DbBackend,
    refresh_id: &str,
    event_sequence: i32,
) -> Result<Option<ExistingLifecycleEvent>, RefreshRepositoryError>
where
    C: ConnectionTrait,
{
    let sql = match backend {
        DatabaseBackend::Postgres => {
            "SELECT event_type, aggregate_type, aggregate_id, refresh_id, event_sequence,
                    payload_version, payload_json, idempotency_key
             FROM lifecycle_outbox WHERE refresh_id=$1 AND event_sequence=$2"
        }
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT event_type, aggregate_type, aggregate_id, refresh_id, event_sequence,
                    payload_version, payload_json, idempotency_key
             FROM lifecycle_outbox WHERE refresh_id=? AND event_sequence=?"
        }
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [refresh_id.into(), event_sequence.into()],
        ))
        .await?
        .map(decode_existing_event)
        .transpose()
}

fn decode_existing_event(
    row: QueryResult,
) -> Result<ExistingLifecycleEvent, RefreshRepositoryError> {
    Ok(ExistingLifecycleEvent {
        event_type: required(&row, "event_type")?,
        aggregate_type: required(&row, "aggregate_type")?,
        aggregate_id: required(&row, "aggregate_id")?,
        refresh_id: required(&row, "refresh_id")?,
        event_sequence: required(&row, "event_sequence")?,
        payload_version: required(&row, "payload_version")?,
        payload_json: required(&row, "payload_json")?,
        idempotency_key: required(&row, "idempotency_key")?,
    })
}

fn required<T: sea_orm::TryGetable>(
    row: &QueryResult,
    column: &str,
) -> Result<T, RefreshRepositoryError> {
    row.try_get("", column)
        .map_err(|_| RefreshRepositoryError::CorruptData)
}

fn insert_event_statement(backend: DbBackend, event: &LifecycleEvent<'_>) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "INSERT INTO lifecycle_outbox (
                id,event_type,aggregate_type,aggregate_id,refresh_id,event_sequence,
                payload_version,payload_json,idempotency_key,available_at,created_at
             ) VALUES (?,?,?,?,?,?,?,?,?,
                strftime('%Y-%m-%dT%H:%M:%f000Z','now'),
                strftime('%Y-%m-%dT%H:%M:%f000Z','now'))"
        }
        DatabaseBackend::Postgres => {
            "INSERT INTO lifecycle_outbox (
                id,event_type,aggregate_type,aggregate_id,refresh_id,event_sequence,
                payload_version,payload_json,idempotency_key,available_at,created_at
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,clock_timestamp(),clock_timestamp())"
        }
        DatabaseBackend::MySql => {
            "INSERT INTO lifecycle_outbox (
                id,event_type,aggregate_type,aggregate_id,refresh_id,event_sequence,
                payload_version,payload_json,idempotency_key,available_at,created_at
             ) VALUES (?,?,?,?,?,?,?,?,?,UTC_TIMESTAMP(6),UTC_TIMESTAMP(6))"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            Uuid::new_v4().to_string().into(),
            event.event_type.into(),
            FEED_AGGREGATE_TYPE.into(),
            event.aggregate_id.into(),
            event.refresh_id.into(),
            event.event_sequence.into(),
            PAYLOAD_VERSION.into(),
            event.payload_json.as_str().into(),
            event.idempotency_key.as_str().into(),
        ],
    )
}

pub(super) fn is_unique_violation(error: &DbErr) -> bool {
    is_unique_violation_provenance(unique_violation_provenance(error))
}

fn unique_violation_provenance(error: &DbErr) -> UniqueViolationProvenance<'_> {
    let runtime = match error {
        DbErr::Conn(runtime) | DbErr::Exec(runtime) | DbErr::Query(runtime) => runtime,
        _ => return UniqueViolationProvenance::Other,
    };
    let RuntimeErr::SqlxError(SqlxError::Database(database_error)) = runtime else {
        return UniqueViolationProvenance::Other;
    };
    if let Some(error) = database_error.try_downcast_ref::<SqlxPostgresError>() {
        return UniqueViolationProvenance::Postgres(error.code());
    }
    if let Some(error) = database_error.try_downcast_ref::<SqlxMySqlError>() {
        return UniqueViolationProvenance::MySql(error.number());
    }
    if database_error
        .try_downcast_ref::<SqlxSqliteError>()
        .is_some()
    {
        return database_error
            .code()
            .as_deref()
            .and_then(|code| code.parse::<i32>().ok())
            .map_or(
                UniqueViolationProvenance::Other,
                UniqueViolationProvenance::Sqlite,
            );
    }
    UniqueViolationProvenance::Other
}

fn is_unique_violation_provenance(provenance: UniqueViolationProvenance<'_>) -> bool {
    match provenance {
        UniqueViolationProvenance::Postgres(code) => code == "23505",
        UniqueViolationProvenance::MySql(number) => number == 1062,
        UniqueViolationProvenance::Sqlite(code) => matches!(code, 1555 | 2067),
        UniqueViolationProvenance::Other => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct OversizedPayload {
        value: String,
    }

    #[test]
    fn unique_violation_codes_are_narrowly_scoped_by_backend() {
        assert!(is_unique_violation_provenance(
            UniqueViolationProvenance::Postgres("23505")
        ));
        assert!(is_unique_violation_provenance(
            UniqueViolationProvenance::MySql(1062)
        ));
        for code in [1555, 2067] {
            assert!(is_unique_violation_provenance(
                UniqueViolationProvenance::Sqlite(code)
            ));
        }
        for code in [19, 275, 787, 1299, 1811] {
            assert!(!is_unique_violation_provenance(
                UniqueViolationProvenance::Sqlite(code)
            ));
        }
    }

    #[test]
    fn typed_payload_encoding_enforces_the_utf8_byte_limit() {
        let error = encode_payload(&OversizedPayload {
            value: "x".repeat(MAX_PAYLOAD_BYTES),
        })
        .expect_err("JSON envelope overhead should exceed the byte limit");
        assert!(matches!(
            error,
            RefreshRepositoryError::LifecyclePayloadTooLarge
        ));
    }
}
