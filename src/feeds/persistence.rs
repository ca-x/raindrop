use std::{
    collections::{HashMap, HashSet},
    fmt,
};

#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicUsize, Ordering};

use sea_orm::{
    ConnectionTrait, DatabaseBackend, DbBackend, DbErr, QueryResult, RuntimeErr, SqlxError,
    SqlxMySqlError, SqlxPostgresError, SqlxSqliteError, Statement, TransactionTrait,
};
use serde::Serialize;
use uuid::Uuid;

use crate::content::sanitize::canonical_summary_text;

use super::{
    EncodedEntryContent, EntryIdentity, FeedRepository, FeedUrlPolicy, OpaqueValidator,
    ParsedEnclosure, ParsedFeed, RefreshClaim, RefreshCounts, RefreshRepositoryError, ValidatorSet,
};

const PIPELINE_VERSION: &str = "sanitize-v1";
const GENERATION_KEY: &str = "INGEST_GENERATION";
const RETRY_BACKOFFS: [std::time::Duration; 2] = [
    std::time::Duration::from_millis(25),
    std::time::Duration::from_millis(50),
];

#[cfg(debug_assertions)]
static PEAK_FULL_EXISTING_ENTRY_BATCH: AtomicUsize = AtomicUsize::new(0);

#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn reset_persistence_batch_observation() {
    PEAK_FULL_EXISTING_ENTRY_BATCH.store(0, Ordering::SeqCst);
}

#[cfg(debug_assertions)]
#[doc(hidden)]
#[must_use]
pub fn persistence_peak_full_existing_entry_batch() -> usize {
    PEAK_FULL_EXISTING_ENTRY_BATCH.load(Ordering::SeqCst)
}

#[cfg(debug_assertions)]
fn observe_full_existing_entry_batch(rows: usize) {
    PEAK_FULL_EXISTING_ENTRY_BATCH.fetch_max(rows, Ordering::SeqCst);
}

#[cfg(not(debug_assertions))]
fn observe_full_existing_entry_batch(_rows: usize) {}

#[derive(Clone)]
pub struct PersistFeed {
    final_url: String,
    etag: Option<OpaqueValidator>,
    last_modified: Option<OpaqueValidator>,
    response_content_hash: String,
    entries: Vec<PersistEntry>,
    dropped_count: i32,
}

impl TryFrom<ParsedFeed> for PersistFeed {
    type Error = RefreshRepositoryError;

    fn try_from(parsed: ParsedFeed) -> Result<Self, Self::Error> {
        let dropped_count = i32::try_from(parsed.duplicate_count)
            .map_err(|_| RefreshRepositoryError::CountOverflow)?;
        let entries = parsed
            .entries
            .into_iter()
            .map(PersistEntry::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            final_url: parsed.source.final_url().to_owned(),
            etag: parsed.source.etag().cloned(),
            last_modified: parsed.source.last_modified().cloned(),
            response_content_hash: hash_hex(*parsed.source.source_document_hash()),
            entries,
            dropped_count,
        })
    }
}

impl fmt::Debug for PersistFeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PersistFeed")
            .field("final_url", &"[REDACTED]")
            .field("etag", &self.etag.as_ref().map(|_| "[REDACTED]"))
            .field(
                "last_modified",
                &self.last_modified.as_ref().map(|_| "[REDACTED]"),
            )
            .field("response_content_hash", &"[PRESENT]")
            .field("entry_count", &self.entries.len())
            .field("dropped_count", &self.dropped_count)
            .finish()
    }
}

#[derive(Clone)]
pub struct PersistEntry {
    id: String,
    identity: EntryIdentity,
    canonical_url: Option<String>,
    title: Option<String>,
    author: Option<String>,
    sanitized_content: EncodedEntryContent,
    summary: Option<String>,
    published_at_us: Option<i64>,
    source_content_hash: String,
    content_hash: String,
    enclosure_json: Option<String>,
}

impl TryFrom<super::ParsedEntry> for PersistEntry {
    type Error = RefreshRepositoryError;

    fn try_from(entry: super::ParsedEntry) -> Result<Self, Self::Error> {
        let published_at_us = entry
            .published_at
            .map(|published| {
                i64::try_from(published.unix_timestamp_nanos() / 1_000)
                    .map_err(|_| RefreshRepositoryError::InvalidTime)
            })
            .transpose()?;
        let sanitized_content = EncodedEntryContent::from_sanitized(&entry.content)
            .map_err(RefreshRepositoryError::Content)?;
        let enclosure_json = encode_enclosures(&entry.enclosures)?;
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            identity: entry.identity,
            canonical_url: entry.canonical_url,
            title: entry.title,
            author: entry.author,
            sanitized_content,
            summary: entry.summary.as_deref().and_then(canonical_summary_text),
            published_at_us,
            source_content_hash: hash_hex(entry.source_content_hash),
            content_hash: hash_hex(entry.content_hash),
            enclosure_json,
        })
    }
}

impl fmt::Debug for PersistEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PersistEntry")
            .field("id", &self.id)
            .field("identity", &self.identity)
            .field(
                "canonical_url",
                &self.canonical_url.as_ref().map(|_| "[REDACTED]"),
            )
            .field("title", &self.title.as_ref().map(|_| "[REDACTED]"))
            .field("author", &self.author.as_ref().map(|_| "[REDACTED]"))
            .field("sanitized_content", &self.sanitized_content)
            .field("summary", &self.summary.as_ref().map(|_| "[REDACTED]"))
            .field("published_at_us", &self.published_at_us)
            .field("source_content_hash", &"[PRESENT]")
            .field("content_hash", &"[PRESENT]")
            .field(
                "enclosure_json",
                &self.enclosure_json.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PersistResult {
    pub counts: RefreshCounts,
    pub generation: Option<i64>,
}

impl FeedRepository {
    pub async fn persist_feed(
        &self,
        claim: &RefreshClaim,
        feed: PersistFeed,
    ) -> Result<PersistResult, RefreshRepositoryError> {
        for backoff in RETRY_BACKOFFS {
            match self.persist_feed_once(claim, &feed).await {
                Err(RefreshRepositoryError::Database(error))
                    if is_transient_database_error(&error) =>
                {
                    tokio::time::sleep(backoff).await;
                }
                result => return result,
            }
        }
        self.persist_feed_once(claim, &feed).await
    }

    async fn persist_feed_once(
        &self,
        claim: &RefreshClaim,
        feed: &PersistFeed,
    ) -> Result<PersistResult, RefreshRepositoryError> {
        validate_claim(claim)?;
        let backend = self.connection().get_database_backend();
        let transaction = self.connection().begin().await?;

        let mut sequence_head = lock_feed(&transaction, backend, &claim.feed_id).await?;
        let authorized = transaction
            .execute(authorize_statement(backend, claim))
            .await?;
        if authorized.rows_affected() != 1 {
            transaction.rollback().await?;
            return Err(RefreshRepositoryError::LeaseLost);
        }
        let insertion_time_us = read_database_time_us(&transaction, backend).await?;

        let existing_hashes =
            lock_existing_identities(&transaction, backend, &claim.feed_id, &feed.entries).await?;

        let new_count = feed
            .entries
            .iter()
            .filter(|entry| !existing_hashes.contains(entry.identity.index_hash()))
            .count();
        let generation = if new_count == 0 {
            None
        } else {
            Some(increment_generation(&transaction, backend).await?)
        };
        let new_count_i64 =
            i64::try_from(new_count).map_err(|_| RefreshRepositoryError::CountOverflow)?;
        let final_sequence_head = sequence_head
            .checked_add(new_count_i64)
            .ok_or(RefreshRepositoryError::SequenceExhausted)?;
        let mut updated_count = 0_i32;

        for entry_batch in feed.entries.chunks(IDENTITY_BATCH_SIZE) {
            let existing =
                lock_existing_entry_batch(&transaction, backend, &claim.feed_id, entry_batch)
                    .await?;
            observe_full_existing_entry_batch(existing.len());
            let mut existing_by_hash = existing
                .into_iter()
                .map(|row| (row.identity_hash.clone(), row))
                .collect::<HashMap<_, _>>();
            for entry in entry_batch {
                if existing_hashes.contains(entry.identity.index_hash()) {
                    let existing = existing_by_hash
                        .remove(entry.identity.index_hash())
                        .ok_or(RefreshRepositoryError::CorruptData)?;
                    if update_existing_entry(&transaction, backend, &existing, entry).await? {
                        updated_count = updated_count
                            .checked_add(1)
                            .ok_or(RefreshRepositoryError::CountOverflow)?;
                    }
                } else {
                    sequence_head = sequence_head
                        .checked_add(1)
                        .ok_or(RefreshRepositoryError::SequenceExhausted)?;
                    insert_entry(
                        &transaction,
                        backend,
                        &claim.feed_id,
                        entry,
                        sequence_head,
                        generation.ok_or(RefreshRepositoryError::CorruptData)?,
                        entry.published_at_us.unwrap_or(insertion_time_us),
                    )
                    .await?;
                }
            }
            if !existing_by_hash.is_empty() {
                transaction.rollback().await?;
                return Err(RefreshRepositoryError::CorruptData);
            }
        }

        let counts = RefreshCounts {
            new_count: i32::try_from(new_count)
                .map_err(|_| RefreshRepositoryError::CountOverflow)?,
            updated_count,
            dropped_count: feed.dropped_count,
        };
        if sequence_head != final_sequence_head {
            transaction.rollback().await?;
            return Err(RefreshRepositoryError::CorruptData);
        }

        let feed_updated = transaction
            .execute(update_feed_statement(
                backend,
                claim,
                feed,
                final_sequence_head,
                counts.new_count != 0 || counts.updated_count != 0,
            ))
            .await?;
        if feed_updated.rows_affected() != 1 {
            transaction.rollback().await?;
            return Err(RefreshRepositoryError::LeaseLost);
        }
        let run_updated = transaction
            .execute(update_run_statement(backend, claim, counts, generation))
            .await?;
        if run_updated.rows_affected() != 1 {
            transaction.rollback().await?;
            return Err(RefreshRepositoryError::LeaseLost);
        }
        let released = transaction
            .execute(release_lease_statement(backend, claim))
            .await?;
        if released.rows_affected() != 1 {
            transaction.rollback().await?;
            return Err(RefreshRepositoryError::LeaseLost);
        }

        transaction.commit().await?;
        Ok(PersistResult { counts, generation })
    }

    pub async fn load_validators(
        &self,
        feed_id: &str,
    ) -> Result<Option<ValidatorSet>, RefreshRepositoryError> {
        if feed_id.is_empty() || feed_id.len() > 36 {
            return Err(RefreshRepositoryError::InvalidRequest);
        }
        let backend = self.connection().get_database_backend();
        let sql = match backend {
            DatabaseBackend::Postgres => {
                "SELECT validator_url, etag, last_modified FROM feeds WHERE id = $1"
            }
            DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                "SELECT validator_url, etag, last_modified FROM feeds WHERE id = ?"
            }
        };
        let Some(row) = self
            .connection()
            .query_one(Statement::from_sql_and_values(
                backend,
                sql,
                [feed_id.into()],
            ))
            .await?
        else {
            return Ok(None);
        };
        let validator_url: Option<String> = row
            .try_get("", "validator_url")
            .map_err(|_| RefreshRepositoryError::CorruptValidator)?;
        let etag: Option<String> = row
            .try_get("", "etag")
            .map_err(|_| RefreshRepositoryError::CorruptValidator)?;
        let last_modified: Option<String> = row
            .try_get("", "last_modified")
            .map_err(|_| RefreshRepositoryError::CorruptValidator)?;
        let Some(validator_url) = validator_url else {
            if etag.is_some() || last_modified.is_some() {
                return Err(RefreshRepositoryError::CorruptValidator);
            }
            return Ok(None);
        };
        let normalized = FeedUrlPolicy::new(true)
            .normalize(&validator_url)
            .map_err(|_| RefreshRepositoryError::CorruptValidator)?;
        let etag = etag
            .as_deref()
            .map(OpaqueValidator::from_storage)
            .transpose()
            .map_err(|_| RefreshRepositoryError::CorruptValidator)?;
        let last_modified = last_modified
            .as_deref()
            .map(OpaqueValidator::from_storage)
            .transpose()
            .map_err(|_| RefreshRepositoryError::CorruptValidator)?;
        Ok(Some(ValidatorSet::new(&normalized, etag, last_modified)))
    }
}

fn is_transient_database_error(error: &DbErr) -> bool {
    is_retryable_provenance(database_error_provenance(error))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetryErrorProvenance<'a> {
    Postgres(&'a str),
    MySql {
        number: u16,
        sqlstate: Option<&'a str>,
    },
    Sqlite(i32),
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InsertErrorDisposition {
    ReturnForRetry,
    ProbeCollision,
}

fn database_error_provenance(error: &DbErr) -> RetryErrorProvenance<'_> {
    let runtime = match error {
        DbErr::Conn(runtime) | DbErr::Exec(runtime) | DbErr::Query(runtime) => runtime,
        _ => return RetryErrorProvenance::Other,
    };
    let RuntimeErr::SqlxError(SqlxError::Database(database_error)) = runtime else {
        return RetryErrorProvenance::Other;
    };
    if let Some(error) = database_error.try_downcast_ref::<SqlxPostgresError>() {
        return RetryErrorProvenance::Postgres(error.code());
    }
    if let Some(error) = database_error.try_downcast_ref::<SqlxMySqlError>() {
        return RetryErrorProvenance::MySql {
            number: error.number(),
            sqlstate: error.code(),
        };
    }
    if database_error
        .try_downcast_ref::<SqlxSqliteError>()
        .is_some()
    {
        return database_error
            .code()
            .as_deref()
            .and_then(|code| code.parse::<i32>().ok())
            .map_or(RetryErrorProvenance::Other, RetryErrorProvenance::Sqlite);
    }
    RetryErrorProvenance::Other
}

fn is_retryable_provenance(provenance: RetryErrorProvenance<'_>) -> bool {
    match provenance {
        RetryErrorProvenance::Postgres(code) => matches!(code.as_bytes(), b"40001" | b"40P01"),
        RetryErrorProvenance::MySql { number, sqlstate } => {
            matches!(number, 1205 | 1213) || matches!(sqlstate, Some("40001"))
        }
        RetryErrorProvenance::Sqlite(code) => code >= 0 && matches!(code & 0xff, 5 | 6),
        RetryErrorProvenance::Other => false,
    }
}

fn insert_error_disposition_for(provenance: RetryErrorProvenance<'_>) -> InsertErrorDisposition {
    if is_retryable_provenance(provenance) {
        InsertErrorDisposition::ReturnForRetry
    } else {
        InsertErrorDisposition::ProbeCollision
    }
}

fn insert_error_disposition(error: &DbErr) -> InsertErrorDisposition {
    insert_error_disposition_for(database_error_provenance(error))
}

#[derive(Debug)]
struct ExistingEntry {
    id: String,
    identity_kind: String,
    identity: String,
    identity_hash: String,
    canonical_url: Option<String>,
    title: Option<String>,
    author: Option<String>,
    sanitized_content: String,
    summary: Option<String>,
    published_at_us: Option<i64>,
    source_content_hash: String,
    content_hash: String,
    pipeline_version: String,
    enclosure_json: Option<String>,
}

const IDENTITY_BATCH_SIZE: usize = 128;

fn validate_claim(claim: &RefreshClaim) -> Result<(), RefreshRepositoryError> {
    if claim.run_id.is_empty()
        || claim.run_id.len() > 36
        || claim.feed_id.is_empty()
        || claim.feed_id.len() > 36
        || claim.owner.is_empty()
        || claim.owner.len() > 128
        || claim.lease_token <= 0
    {
        return Err(RefreshRepositoryError::InvalidRequest);
    }
    Ok(())
}

async fn lock_feed<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
) -> Result<i64, RefreshRepositoryError> {
    let sql = match backend {
        DatabaseBackend::Sqlite => "SELECT entry_sequence_head FROM feeds WHERE id = ?",
        DatabaseBackend::Postgres => {
            "SELECT entry_sequence_head FROM feeds WHERE id = $1 FOR UPDATE"
        }
        DatabaseBackend::MySql => "SELECT entry_sequence_head FROM feeds WHERE id = ? FOR UPDATE",
    };
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into()],
        ))
        .await?
        .ok_or(RefreshRepositoryError::LeaseLost)?;
    let head: i64 = row
        .try_get("", "entry_sequence_head")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    if head < 0 {
        return Err(RefreshRepositoryError::CorruptData);
    }
    Ok(head)
}

fn authorize_statement(backend: DbBackend, claim: &RefreshClaim) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feeds SET updated_at = strftime('%Y-%m-%dT%H:%M:%f000Z', 'now')
             WHERE id = ? AND lease_owner = ? AND lease_token = ? AND lease_until IS NOT NULL
               AND julianday(lease_until) > julianday('now')
               AND EXISTS (SELECT 1 FROM feed_refresh_runs r
                   WHERE r.id = ? AND r.feed_id = feeds.id AND r.status = 'RUNNING'
                     AND r.lease_token = feeds.lease_token)"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feeds SET updated_at = clock_timestamp()
             WHERE id = $1 AND lease_owner = $2 AND lease_token = $3 AND lease_until IS NOT NULL
               AND lease_until > clock_timestamp()
               AND EXISTS (SELECT 1 FROM feed_refresh_runs r
                   WHERE r.id = $4 AND r.feed_id = feeds.id AND r.status = 'RUNNING'
                     AND r.lease_token = feeds.lease_token)"
        }
        DatabaseBackend::MySql => {
            "UPDATE feeds SET updated_at = UTC_TIMESTAMP(6)
             WHERE id = ? AND lease_owner = ? AND lease_token = ? AND lease_until IS NOT NULL
               AND lease_until > UTC_TIMESTAMP(6)
               AND EXISTS (SELECT 1 FROM feed_refresh_runs r
                   WHERE r.id = ? AND r.feed_id = feeds.id AND r.status = 'RUNNING'
                     AND r.lease_token = feeds.lease_token)"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            claim.feed_id.as_str().into(),
            claim.owner.as_str().into(),
            claim.lease_token.into(),
            claim.run_id.as_str().into(),
        ],
    )
}

async fn read_database_time_us<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
) -> Result<i64, RefreshRepositoryError> {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "SELECT CAST((julianday('now') - 2440587.5) * 86400000000 AS INTEGER) AS now_us"
        }
        DatabaseBackend::Postgres => {
            "SELECT CAST(EXTRACT(EPOCH FROM clock_timestamp()) * 1000000 AS BIGINT) AS now_us"
        }
        DatabaseBackend::MySql => {
            "SELECT TIMESTAMPDIFF(MICROSECOND, '1970-01-01 00:00:00', UTC_TIMESTAMP(6)) AS now_us"
        }
    };
    let row = connection
        .query_one(Statement::from_string(backend, sql.to_owned()))
        .await?
        .ok_or(RefreshRepositoryError::InvalidTime)?;
    row.try_get("", "now_us")
        .map_err(|_| RefreshRepositoryError::InvalidTime)
}

async fn lock_existing_identities<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
    entries: &[PersistEntry],
) -> Result<HashSet<String>, RefreshRepositoryError> {
    let mut identity_hashes = entries
        .iter()
        .map(|entry| entry.identity.index_hash())
        .collect::<Vec<_>>();
    identity_hashes.sort_unstable();
    identity_hashes.dedup();
    let incoming = entries
        .iter()
        .map(|entry| (entry.identity.index_hash(), &entry.identity))
        .collect::<HashMap<_, _>>();
    let mut existing_hashes = HashSet::with_capacity(identity_hashes.len());
    for batch in identity_hashes.chunks(IDENTITY_BATCH_SIZE) {
        let rows = connection
            .query_all(identity_lookup_statement(backend, feed_id, batch, false))
            .await?;
        for row in rows {
            let identity_kind: String = required(&row, "identity_kind")?;
            let identity: String = required(&row, "identity")?;
            let identity_hash: String = required(&row, "identity_hash")?;
            let incoming_identity = incoming
                .get(identity_hash.as_str())
                .ok_or(RefreshRepositoryError::CorruptData)?;
            ensure_identity_parts(&identity_kind, &identity, incoming_identity)?;
            existing_hashes.insert(identity_hash);
        }
    }
    Ok(existing_hashes)
}

async fn lock_existing_entry_batch<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
    entries: &[PersistEntry],
) -> Result<Vec<ExistingEntry>, RefreshRepositoryError> {
    let mut identity_hashes = entries
        .iter()
        .map(|entry| entry.identity.index_hash())
        .collect::<Vec<_>>();
    identity_hashes.sort_unstable();
    identity_hashes.dedup();
    connection
        .query_all(identity_lookup_statement(
            backend,
            feed_id,
            &identity_hashes,
            true,
        ))
        .await?
        .into_iter()
        .map(decode_existing_entry)
        .collect()
}

fn identity_lookup_statement(
    backend: DbBackend,
    feed_id: &str,
    identity_hashes: &[&str],
    full: bool,
) -> Statement {
    let mut values = Vec::with_capacity(identity_hashes.len() + 1);
    values.push(feed_id.into());
    values.extend(identity_hashes.iter().map(|hash| (*hash).into()));
    let (feed_placeholder, hash_placeholders) = if backend == DatabaseBackend::Postgres {
        (
            "$1".to_owned(),
            (2..=identity_hashes.len() + 1)
                .map(|index| format!("${index}"))
                .collect::<Vec<_>>()
                .join(","),
        )
    } else {
        ("?".to_owned(), vec!["?"; identity_hashes.len()].join(","))
    };
    let columns = if full {
        "id, identity_kind, identity, identity_hash, canonical_url, title, author,
         sanitized_content, summary, published_at_us, source_content_hash, content_hash,
         pipeline_version, enclosure_json"
    } else {
        "identity_kind, identity, identity_hash"
    };
    let lock = if backend == DatabaseBackend::Sqlite {
        ""
    } else {
        " FOR UPDATE"
    };
    let sql = format!(
        "SELECT {columns} FROM entries WHERE feed_id = {feed_placeholder}
         AND identity_hash IN ({hash_placeholders}) ORDER BY identity_hash{lock}"
    );
    Statement::from_sql_and_values(backend, sql, values)
}

fn decode_existing_entry(row: QueryResult) -> Result<ExistingEntry, RefreshRepositoryError> {
    Ok(ExistingEntry {
        id: required(&row, "id")?,
        identity_kind: required(&row, "identity_kind")?,
        identity: required(&row, "identity")?,
        identity_hash: required(&row, "identity_hash")?,
        canonical_url: optional(&row, "canonical_url")?,
        title: optional(&row, "title")?,
        author: optional(&row, "author")?,
        sanitized_content: required(&row, "sanitized_content")?,
        summary: optional(&row, "summary")?,
        published_at_us: optional(&row, "published_at_us")?,
        source_content_hash: required(&row, "source_content_hash")?,
        content_hash: required(&row, "content_hash")?,
        pipeline_version: required(&row, "pipeline_version")?,
        enclosure_json: optional(&row, "enclosure_json")?,
    })
}

fn required<T: sea_orm::TryGetable>(
    row: &QueryResult,
    column: &str,
) -> Result<T, RefreshRepositoryError> {
    row.try_get("", column)
        .map_err(|_| RefreshRepositoryError::CorruptData)
}

fn optional<T: sea_orm::TryGetable>(
    row: &QueryResult,
    column: &str,
) -> Result<Option<T>, RefreshRepositoryError> {
    row.try_get("", column)
        .map_err(|_| RefreshRepositoryError::CorruptData)
}

fn ensure_same_identity(
    existing: &ExistingEntry,
    identity: &EntryIdentity,
) -> Result<(), RefreshRepositoryError> {
    ensure_identity_parts(&existing.identity_kind, &existing.identity, identity)
}

fn ensure_identity_parts(
    identity_kind: &str,
    persisted_identity: &str,
    identity: &EntryIdentity,
) -> Result<(), RefreshRepositoryError> {
    if identity_kind != identity.kind().as_database_str()
        || persisted_identity != identity.identity()
    {
        Err(RefreshRepositoryError::IdentityHashCollision)
    } else {
        Ok(())
    }
}

async fn increment_generation<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
) -> Result<i64, RefreshRepositoryError> {
    let sql = match backend {
        DatabaseBackend::Sqlite => "SELECT value FROM rss_counters WHERE key = ?",
        DatabaseBackend::Postgres => "SELECT value FROM rss_counters WHERE key = $1 FOR UPDATE",
        DatabaseBackend::MySql => "SELECT value FROM rss_counters WHERE `key` = ? FOR UPDATE",
    };
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [GENERATION_KEY.into()],
        ))
        .await?
        .ok_or(RefreshRepositoryError::CorruptData)?;
    let current: i64 = row
        .try_get("", "value")
        .map_err(|_| RefreshRepositoryError::CorruptData)?;
    let next = current
        .checked_add(1)
        .filter(|value| *value > 0)
        .ok_or(RefreshRepositoryError::GenerationExhausted)?;
    let update = match backend {
        DatabaseBackend::Postgres => Statement::from_sql_and_values(
            backend,
            "UPDATE rss_counters SET value = $1 WHERE key = $2 AND value = $3",
            [next.into(), GENERATION_KEY.into(), current.into()],
        ),
        DatabaseBackend::Sqlite => Statement::from_sql_and_values(
            backend,
            "UPDATE rss_counters SET value = ? WHERE key = ? AND value = ?",
            [next.into(), GENERATION_KEY.into(), current.into()],
        ),
        DatabaseBackend::MySql => Statement::from_sql_and_values(
            backend,
            "UPDATE rss_counters SET value = ? WHERE `key` = ? AND value = ?",
            [next.into(), GENERATION_KEY.into(), current.into()],
        ),
    };
    if connection.execute(update).await?.rows_affected() != 1 {
        return Err(RefreshRepositoryError::CorruptData);
    }
    Ok(next)
}

async fn update_existing_entry<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    existing: &ExistingEntry,
    entry: &PersistEntry,
) -> Result<bool, RefreshRepositoryError> {
    ensure_same_identity(existing, &entry.identity)?;
    let metadata_changed = existing.canonical_url != entry.canonical_url
        || existing.title != entry.title
        || existing.author != entry.author
        || existing.summary != entry.summary
        || existing.published_at_us != entry.published_at_us
        || existing.enclosure_json != entry.enclosure_json;
    let envelope_changed = existing.sanitized_content != entry.sanitized_content.as_storage_str();
    let hashes_changed = existing.source_content_hash != entry.source_content_hash
        || existing.content_hash != entry.content_hash
        || existing.pipeline_version != PIPELINE_VERSION;
    if !metadata_changed && !envelope_changed && !hashes_changed {
        return Ok(false);
    }

    let statement = if hashes_changed {
        update_entry_with_hashes_statement(backend, existing, entry)
    } else if envelope_changed {
        update_entry_with_envelope_statement(backend, existing, entry)
    } else {
        update_entry_metadata_statement(backend, existing, entry)
    };
    if connection.execute(statement).await?.rows_affected() != 1 {
        return Err(RefreshRepositoryError::CorruptData);
    }
    Ok(true)
}

fn update_entry_metadata_statement(
    backend: DbBackend,
    existing: &ExistingEntry,
    entry: &PersistEntry,
) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE entries SET canonical_url=?, title=?, author=?, summary=?, published_at_us=?,
                enclosure_json=?, updated_at=strftime('%Y-%m-%dT%H:%M:%f000Z','now') WHERE id=?"
        }
        DatabaseBackend::Postgres => {
            "UPDATE entries SET canonical_url=$1, title=$2, author=$3, summary=$4,
                published_at_us=$5, enclosure_json=$6, updated_at=clock_timestamp() WHERE id=$7"
        }
        DatabaseBackend::MySql => {
            "UPDATE entries SET canonical_url=?, title=?, author=?, summary=?, published_at_us=?,
                enclosure_json=?, updated_at=UTC_TIMESTAMP(6) WHERE id=?"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            entry.canonical_url.as_deref().into(),
            entry.title.as_deref().into(),
            entry.author.as_deref().into(),
            entry.summary.as_deref().into(),
            entry.published_at_us.into(),
            entry.enclosure_json.as_deref().into(),
            existing.id.as_str().into(),
        ],
    )
}

fn update_entry_with_envelope_statement(
    backend: DbBackend,
    existing: &ExistingEntry,
    entry: &PersistEntry,
) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE entries SET canonical_url=?, title=?, author=?, sanitized_content=?, summary=?,
                published_at_us=?, enclosure_json=?,
                updated_at=strftime('%Y-%m-%dT%H:%M:%f000Z','now') WHERE id=?"
        }
        DatabaseBackend::Postgres => {
            "UPDATE entries SET canonical_url=$1, title=$2, author=$3, sanitized_content=$4,
                summary=$5, published_at_us=$6, enclosure_json=$7,
                updated_at=clock_timestamp() WHERE id=$8"
        }
        DatabaseBackend::MySql => {
            "UPDATE entries SET canonical_url=?, title=?, author=?, sanitized_content=?, summary=?,
                published_at_us=?, enclosure_json=?, updated_at=UTC_TIMESTAMP(6) WHERE id=?"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            entry.canonical_url.as_deref().into(),
            entry.title.as_deref().into(),
            entry.author.as_deref().into(),
            entry.sanitized_content.as_storage_str().into(),
            entry.summary.as_deref().into(),
            entry.published_at_us.into(),
            entry.enclosure_json.as_deref().into(),
            existing.id.as_str().into(),
        ],
    )
}

fn update_entry_with_hashes_statement(
    backend: DbBackend,
    existing: &ExistingEntry,
    entry: &PersistEntry,
) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE entries SET canonical_url=?, title=?, author=?, sanitized_content=?, summary=?,
                published_at_us=?, source_content_hash=?, content_hash=?, pipeline_version=?,
                enclosure_json=?, updated_at=strftime('%Y-%m-%dT%H:%M:%f000Z','now') WHERE id=?"
        }
        DatabaseBackend::Postgres => {
            "UPDATE entries SET canonical_url=$1, title=$2, author=$3, sanitized_content=$4,
                summary=$5, published_at_us=$6, source_content_hash=$7, content_hash=$8,
                pipeline_version=$9, enclosure_json=$10, updated_at=clock_timestamp() WHERE id=$11"
        }
        DatabaseBackend::MySql => {
            "UPDATE entries SET canonical_url=?, title=?, author=?, sanitized_content=?, summary=?,
                published_at_us=?, source_content_hash=?, content_hash=?, pipeline_version=?,
                enclosure_json=?, updated_at=UTC_TIMESTAMP(6) WHERE id=?"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            entry.canonical_url.as_deref().into(),
            entry.title.as_deref().into(),
            entry.author.as_deref().into(),
            entry.sanitized_content.as_storage_str().into(),
            entry.summary.as_deref().into(),
            entry.published_at_us.into(),
            entry.source_content_hash.as_str().into(),
            entry.content_hash.as_str().into(),
            PIPELINE_VERSION.into(),
            entry.enclosure_json.as_deref().into(),
            existing.id.as_str().into(),
        ],
    )
}

async fn insert_entry<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
    entry: &PersistEntry,
    sequence: i64,
    generation: i64,
    sort_at_us: i64,
) -> Result<(), RefreshRepositoryError> {
    let result = connection
        .execute(insert_entry_statement(
            backend, feed_id, entry, sequence, generation, sort_at_us,
        ))
        .await;
    match result {
        Ok(result) if result.rows_affected() == 1 => Ok(()),
        Ok(_) => Err(RefreshRepositoryError::CorruptData),
        Err(error) => {
            if insert_error_disposition(&error) == InsertErrorDisposition::ReturnForRetry {
                return Err(RefreshRepositoryError::Database(error));
            }
            let existing = find_entry_by_identity_hash(
                connection,
                backend,
                feed_id,
                entry.identity.index_hash(),
            )
            .await?;
            if let Some(existing) = existing {
                ensure_same_identity(&existing, &entry.identity)?;
                Err(RefreshRepositoryError::Database(error))
            } else {
                Err(RefreshRepositoryError::Database(error))
            }
        }
    }
}

fn insert_entry_statement(
    backend: DbBackend,
    feed_id: &str,
    entry: &PersistEntry,
    sequence: i64,
    generation: i64,
    sort_at_us: i64,
) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "INSERT INTO entries (id,feed_id,feed_sequence,ingest_generation,identity_kind,identity,
                identity_hash,canonical_url,title,author,sanitized_content,summary,published_at_us,
                sort_at_us,inserted_at,updated_at,source_content_hash,content_hash,pipeline_version,
                direction,enclosure_json)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%f000Z','now'),
                strftime('%Y-%m-%dT%H:%M:%f000Z','now'),?,?,?,?,?)"
        }
        DatabaseBackend::Postgres => {
            "INSERT INTO entries (id,feed_id,feed_sequence,ingest_generation,identity_kind,identity,
                identity_hash,canonical_url,title,author,sanitized_content,summary,published_at_us,
                sort_at_us,inserted_at,updated_at,source_content_hash,content_hash,pipeline_version,
                direction,enclosure_json)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,clock_timestamp(),
                clock_timestamp(),$15,$16,$17,$18,$19)"
        }
        DatabaseBackend::MySql => {
            "INSERT INTO entries (id,feed_id,feed_sequence,ingest_generation,identity_kind,identity,
                identity_hash,canonical_url,title,author,sanitized_content,summary,published_at_us,
                sort_at_us,inserted_at,updated_at,source_content_hash,content_hash,pipeline_version,
                direction,enclosure_json)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,UTC_TIMESTAMP(6),UTC_TIMESTAMP(6),?,?,?,?,?)"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            entry.id.as_str().into(),
            feed_id.into(),
            sequence.into(),
            generation.into(),
            entry.identity.kind().as_database_str().into(),
            entry.identity.identity().into(),
            entry.identity.index_hash().into(),
            entry.canonical_url.as_deref().into(),
            entry.title.as_deref().into(),
            entry.author.as_deref().into(),
            entry.sanitized_content.as_storage_str().into(),
            entry.summary.as_deref().into(),
            entry.published_at_us.into(),
            sort_at_us.into(),
            entry.source_content_hash.as_str().into(),
            entry.content_hash.as_str().into(),
            PIPELINE_VERSION.into(),
            Option::<&str>::None.into(),
            entry.enclosure_json.as_deref().into(),
        ],
    )
}

async fn find_entry_by_identity_hash<C: ConnectionTrait>(
    connection: &C,
    backend: DbBackend,
    feed_id: &str,
    identity_hash: &str,
) -> Result<Option<ExistingEntry>, RefreshRepositoryError> {
    let sql = match backend {
        DatabaseBackend::Postgres => {
            "SELECT id, identity_kind, identity, identity_hash, canonical_url, title, author,
                sanitized_content, summary, published_at_us, source_content_hash, content_hash,
                pipeline_version, enclosure_json FROM entries
             WHERE feed_id=$1 AND identity_hash=$2"
        }
        DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
            "SELECT id, identity_kind, identity, identity_hash, canonical_url, title, author,
                sanitized_content, summary, published_at_us, source_content_hash, content_hash,
                pipeline_version, enclosure_json FROM entries
             WHERE feed_id=? AND identity_hash=?"
        }
    };
    connection
        .query_one(Statement::from_sql_and_values(
            backend,
            sql,
            [feed_id.into(), identity_hash.into()],
        ))
        .await?
        .map(decode_existing_entry)
        .transpose()
}

fn update_feed_statement(
    backend: DbBackend,
    claim: &RefreshClaim,
    feed: &PersistFeed,
    sequence_head: i64,
    changed: bool,
) -> Statement {
    let changed_assignment = if changed {
        match backend {
            DatabaseBackend::Sqlite => "last_changed_at=strftime('%Y-%m-%dT%H:%M:%f000Z','now'),",
            DatabaseBackend::Postgres => "last_changed_at=clock_timestamp(),",
            DatabaseBackend::MySql => "last_changed_at=UTC_TIMESTAMP(6),",
        }
    } else {
        ""
    };
    let (clock, placeholders) = match backend {
        DatabaseBackend::Sqlite => (
            "strftime('%Y-%m-%dT%H:%M:%f000Z','now')",
            ["?", "?", "?", "?", "?", "?", "?", "?", "?"],
        ),
        DatabaseBackend::Postgres => (
            "clock_timestamp()",
            ["$1", "$2", "$3", "$4", "$5", "$6", "$7", "$8", "$9"],
        ),
        DatabaseBackend::MySql => (
            "UTC_TIMESTAMP(6)",
            ["?", "?", "?", "?", "?", "?", "?", "?", "?"],
        ),
    };
    let sql = format!(
        "UPDATE feeds SET fetch_url={0}, validator_url={1}, etag={2}, last_modified={3},
            response_content_hash={4}, entry_sequence_head={5}, last_attempt_at={clock},
            last_success_at={clock}, {changed_assignment} retry_after_at=NULL,
            consecutive_failures=0, last_error_code=NULL, orphaned_at=NULL, updated_at={clock}
         WHERE id={6} AND lease_owner={7} AND lease_token={8}",
        placeholders[0],
        placeholders[1],
        placeholders[2],
        placeholders[3],
        placeholders[4],
        placeholders[5],
        placeholders[6],
        placeholders[7],
        placeholders[8]
    );
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            feed.final_url.as_str().into(),
            feed.final_url.as_str().into(),
            feed.etag
                .as_ref()
                .map(OpaqueValidator::storage_value)
                .into(),
            feed.last_modified
                .as_ref()
                .map(OpaqueValidator::storage_value)
                .into(),
            feed.response_content_hash.as_str().into(),
            sequence_head.into(),
            claim.feed_id.as_str().into(),
            claim.owner.as_str().into(),
            claim.lease_token.into(),
        ],
    )
}

fn update_run_statement(
    backend: DbBackend,
    claim: &RefreshClaim,
    counts: RefreshCounts,
    generation: Option<i64>,
) -> Statement {
    let status = if counts.dropped_count == 0 {
        "SUCCESS"
    } else {
        "PARTIAL"
    };
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feed_refresh_runs SET status=?, commit_generation=?,
                fetched_at=strftime('%Y-%m-%dT%H:%M:%f000Z','now'),
                persisted_at=strftime('%Y-%m-%dT%H:%M:%f000Z','now'),
                completed_at=strftime('%Y-%m-%dT%H:%M:%f000Z','now'), http_status=200,
                new_count=?, updated_count=?, dropped_count=?, error_code=NULL, retry_at=NULL
             WHERE id=? AND feed_id=? AND status='RUNNING' AND lease_token=?"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feed_refresh_runs SET status=$1, commit_generation=$2,
                fetched_at=clock_timestamp(), persisted_at=clock_timestamp(),
                completed_at=clock_timestamp(), http_status=200, new_count=$3,
                updated_count=$4, dropped_count=$5, error_code=NULL, retry_at=NULL
             WHERE id=$6 AND feed_id=$7 AND status='RUNNING' AND lease_token=$8"
        }
        DatabaseBackend::MySql => {
            "UPDATE feed_refresh_runs SET status=?, commit_generation=?, fetched_at=UTC_TIMESTAMP(6),
                persisted_at=UTC_TIMESTAMP(6), completed_at=UTC_TIMESTAMP(6), http_status=200,
                new_count=?, updated_count=?, dropped_count=?, error_code=NULL, retry_at=NULL
             WHERE id=? AND feed_id=? AND status='RUNNING' AND lease_token=?"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            status.into(),
            generation.into(),
            counts.new_count.into(),
            counts.updated_count.into(),
            counts.dropped_count.into(),
            claim.run_id.as_str().into(),
            claim.feed_id.as_str().into(),
            claim.lease_token.into(),
        ],
    )
}

fn release_lease_statement(backend: DbBackend, claim: &RefreshClaim) -> Statement {
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "UPDATE feeds SET lease_owner=NULL, lease_until=NULL,
                updated_at=strftime('%Y-%m-%dT%H:%M:%f000Z','now')
             WHERE id=? AND lease_owner=? AND lease_token=?"
        }
        DatabaseBackend::Postgres => {
            "UPDATE feeds SET lease_owner=NULL, lease_until=NULL, updated_at=clock_timestamp()
             WHERE id=$1 AND lease_owner=$2 AND lease_token=$3"
        }
        DatabaseBackend::MySql => {
            "UPDATE feeds SET lease_owner=NULL, lease_until=NULL, updated_at=UTC_TIMESTAMP(6)
             WHERE id=? AND lease_owner=? AND lease_token=?"
        }
    };
    Statement::from_sql_and_values(
        backend,
        sql,
        [
            claim.feed_id.as_str().into(),
            claim.owner.as_str().into(),
            claim.lease_token.into(),
        ],
    )
}

#[derive(Serialize)]
struct EnclosureEnvelope<'a> {
    version: u8,
    items: Vec<StoredEnclosure<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredEnclosure<'a> {
    url: &'a str,
    media_type: Option<&'a str>,
    length: Option<&'a str>,
    title: Option<&'a str>,
    duration: Option<&'a str>,
}

fn encode_enclosures(
    enclosures: &[ParsedEnclosure],
) -> Result<Option<String>, RefreshRepositoryError> {
    if enclosures.is_empty() {
        return Ok(None);
    }
    let envelope = EnclosureEnvelope {
        version: 1,
        items: enclosures
            .iter()
            .map(|enclosure| StoredEnclosure {
                url: enclosure.url(),
                media_type: enclosure.media_type(),
                length: enclosure.length(),
                title: enclosure.title(),
                duration: enclosure.duration(),
            })
            .collect(),
    };
    serde_json::to_string(&envelope)
        .map(Some)
        .map_err(|_| RefreshRepositoryError::InvalidContent)
}

fn hash_hex(bytes: [u8; 32]) -> String {
    blake3::Hash::from_bytes(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_insert_transients_return_before_collision_probe() {
        for code in ["40001", "40P01"] {
            assert_eq!(
                insert_error_disposition_for(RetryErrorProvenance::Postgres(code)),
                InsertErrorDisposition::ReturnForRetry
            );
        }
    }

    #[test]
    fn retry_codes_are_whitelisted_by_backend_provenance() {
        for code in ["22021", "22022"] {
            assert_eq!(
                insert_error_disposition_for(RetryErrorProvenance::Postgres(code)),
                InsertErrorDisposition::ProbeCollision
            );
        }
        for code in [5, 6, 261, 262, 517, 518] {
            assert!(is_retryable_provenance(RetryErrorProvenance::Sqlite(code)));
        }
        for number in [1205, 1213] {
            assert!(is_retryable_provenance(RetryErrorProvenance::MySql {
                number,
                sqlstate: None,
            }));
        }
        assert!(is_retryable_provenance(RetryErrorProvenance::MySql {
            number: 0,
            sqlstate: Some("40001"),
        }));
        assert!(!is_retryable_provenance(RetryErrorProvenance::Other));
    }
}
