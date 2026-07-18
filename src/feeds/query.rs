use std::fmt;

use base64::Engine;
use sea_orm::{ConnectionTrait, DatabaseBackend, DbBackend, QueryResult, Statement, Value};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    EnclosureDto, EntryContentDetail, EntryContentError, EntryDetailDto, EntryListItemDto,
    EntryPage, FeedRepository, InertImageDto,
};

const CURSOR_VERSION: u8 = 1;
const MAX_CURSOR_BYTES: usize = 1_024;
const MAX_LIMIT: u16 = 100;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum EntryListState {
    All,
    #[default]
    Unread,
    Starred,
}

impl EntryListState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::Unread => "UNREAD",
            Self::Starred => "STARRED",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListEntriesQuery {
    pub state: EntryListState,
    pub feed_id: Option<String>,
    pub category_id: Option<String>,
    pub limit: u16,
    pub cursor: Option<String>,
}

impl Default for ListEntriesQuery {
    fn default() -> Self {
        Self {
            state: EntryListState::Unread,
            feed_id: None,
            category_id: None,
            limit: 50,
            cursor: None,
        }
    }
}

#[derive(thiserror::Error)]
pub enum RepositoryError {
    #[error("entry repository database operation failed")]
    Database(#[source] sea_orm::DbErr),
    #[error("user identifier is invalid")]
    InvalidUserId,
    #[error("feed identifier is invalid")]
    InvalidFeedId,
    #[error("category identifier is invalid")]
    InvalidCategoryId,
    #[error("entry source filters cannot be combined")]
    InvalidSourceFilter,
    #[error("entry identifier is invalid")]
    InvalidEntryId,
    #[error("entry list limit is invalid")]
    InvalidLimit,
    #[error("entry list cursor is invalid")]
    InvalidCursor,
    #[error("entry state patch is empty")]
    InvalidStatePatch,
    #[error("entry repository data is corrupt")]
    CorruptData,
    #[error("stored entry content is invalid")]
    Content(#[source] EntryContentError),
}

impl fmt::Debug for RepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Database(_) => "RepositoryError::Database([REDACTED])",
            Self::InvalidUserId => "RepositoryError::InvalidUserId",
            Self::InvalidFeedId => "RepositoryError::InvalidFeedId",
            Self::InvalidCategoryId => "RepositoryError::InvalidCategoryId",
            Self::InvalidSourceFilter => "RepositoryError::InvalidSourceFilter",
            Self::InvalidEntryId => "RepositoryError::InvalidEntryId",
            Self::InvalidLimit => "RepositoryError::InvalidLimit",
            Self::InvalidCursor => "RepositoryError::InvalidCursor",
            Self::InvalidStatePatch => "RepositoryError::InvalidStatePatch",
            Self::CorruptData => "RepositoryError::CorruptData",
            Self::Content(_) => "RepositoryError::Content([REDACTED])",
        })
    }
}

impl From<sea_orm::DbErr> for RepositoryError {
    fn from(error: sea_orm::DbErr) -> Self {
        Self::Database(error)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CursorPayload {
    v: u8,
    filter_hash: String,
    snapshot_generation: i64,
    sort_at_us: i64,
    entry_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EnclosureEnvelope {
    version: u8,
    items: Vec<EnclosureDto>,
}

impl FeedRepository {
    pub async fn list_for_user(
        &self,
        user_id: &str,
        query: ListEntriesQuery,
    ) -> Result<EntryPage, RepositoryError> {
        validate_uuid(user_id).map_err(|()| RepositoryError::InvalidUserId)?;
        validate_query(&query)?;
        let filter_hash = filter_hash(
            user_id,
            query.state,
            query.feed_id.as_deref(),
            query.category_id.as_deref(),
        );
        let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.filter_hash != filter_hash)
        {
            return Err(RepositoryError::InvalidCursor);
        }

        let backend = self.connection().get_database_backend();
        let snapshot_generation = match cursor.as_ref() {
            Some(cursor) => cursor.snapshot_generation,
            None => read_snapshot_generation(self.connection(), backend).await?,
        };
        let statement = list_statement(
            backend,
            user_id,
            &query,
            snapshot_generation,
            cursor.as_ref(),
        );
        let rows = self.connection().query_all(statement).await?;
        let mut items = rows
            .into_iter()
            .map(decode_list_item)
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = items.len() > usize::from(query.limit);
        if has_more {
            items.pop();
        }
        let next_cursor = if has_more {
            let last = items.last().ok_or(RepositoryError::CorruptData)?;
            Some(encode_cursor(&CursorPayload {
                v: CURSOR_VERSION,
                filter_hash,
                snapshot_generation,
                sort_at_us: last.sort_at_us,
                entry_id: last.entry_id.clone(),
            })?)
        } else {
            None
        };

        Ok(EntryPage {
            items,
            next_cursor,
            snapshot_generation,
        })
    }

    pub async fn get_detail_for_user(
        &self,
        user_id: &str,
        entry_id: &str,
    ) -> Result<Option<EntryDetailDto>, RepositoryError> {
        validate_uuid(user_id).map_err(|()| RepositoryError::InvalidUserId)?;
        validate_uuid(entry_id).map_err(|()| RepositoryError::InvalidEntryId)?;
        let backend = self.connection().get_database_backend();
        self.connection()
            .query_one(detail_statement(backend, user_id, entry_id))
            .await?
            .map(decode_detail)
            .transpose()
    }

    #[doc(hidden)]
    pub async fn explain_list_for_user(
        &self,
        user_id: &str,
        query: ListEntriesQuery,
    ) -> Result<Vec<String>, RepositoryError> {
        validate_uuid(user_id).map_err(|()| RepositoryError::InvalidUserId)?;
        validate_query(&query)?;
        let backend = self.connection().get_database_backend();
        let snapshot_generation = read_snapshot_generation(self.connection(), backend).await?;
        let statement = list_statement(backend, user_id, &query, snapshot_generation, None);
        let explain = explain_statement(statement);
        self.connection()
            .query_all(explain)
            .await?
            .into_iter()
            .map(|row| explain_line(backend, &row))
            .collect()
    }
}

fn validate_query(query: &ListEntriesQuery) -> Result<(), RepositoryError> {
    if !(1..=MAX_LIMIT).contains(&query.limit) {
        return Err(RepositoryError::InvalidLimit);
    }
    if let Some(feed_id) = query.feed_id.as_deref() {
        validate_uuid(feed_id).map_err(|()| RepositoryError::InvalidFeedId)?;
    }
    if let Some(category_id) = query.category_id.as_deref() {
        validate_uuid(category_id).map_err(|()| RepositoryError::InvalidCategoryId)?;
    }
    if query.feed_id.is_some() && query.category_id.is_some() {
        return Err(RepositoryError::InvalidSourceFilter);
    }
    Ok(())
}

pub(super) fn validate_uuid(value: &str) -> Result<(), ()> {
    let parsed = Uuid::parse_str(value).map_err(|_| ())?;
    (parsed.to_string() == value).then_some(()).ok_or(())
}

async fn read_snapshot_generation<C>(
    connection: &C,
    backend: DbBackend,
) -> Result<i64, RepositoryError>
where
    C: ConnectionTrait,
{
    let row = connection
        .query_one(Statement::from_sql_and_values(
            backend,
            match backend {
                DatabaseBackend::Postgres => "SELECT value FROM rss_counters WHERE key = $1",
                DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                    "SELECT value FROM rss_counters WHERE key = ?"
                }
            },
            ["INGEST_GENERATION".into()],
        ))
        .await?
        .ok_or(RepositoryError::CorruptData)?;
    let value: i64 = required(&row, "value")?;
    if value < 0 {
        return Err(RepositoryError::CorruptData);
    }
    Ok(value)
}

fn list_statement(
    backend: DbBackend,
    user_id: &str,
    query: &ListEntriesQuery,
    snapshot_generation: i64,
    cursor: Option<&CursorPayload>,
) -> Statement {
    let mut sql = Sql::new(backend);
    let user = sql.bind(user_id);
    let snapshot = sql.bind(snapshot_generation);
    let state_join = match query.state {
        EntryListState::Starred => {
            "JOIN entry_states es ON es.user_id = s.user_id AND es.entry_id = e.id
                                  AND es.feed_id = e.feed_id
                                  AND es.feed_sequence = e.feed_sequence
                                  AND es.is_starred = TRUE"
        }
        EntryListState::All | EntryListState::Unread => {
            "LEFT JOIN entry_states es ON es.user_id = s.user_id AND es.entry_id = e.id
                                       AND es.feed_id = e.feed_id
                                       AND es.feed_sequence = e.feed_sequence"
        }
    };
    let mut text = format!(
        "SELECT e.id AS entry_id, e.feed_id AS feed_id, f.title AS feed_title,
                f.site_url AS site_url, f.normalized_url AS feed_url, e.title AS entry_title,
                e.author AS author, e.summary AS summary, e.canonical_url AS canonical_url,
                e.published_at_us AS published_at_us, e.sort_at_us AS sort_at_us,
                CASE WHEN es.read_override IS NOT NULL THEN es.read_override
                     ELSE e.feed_sequence <= s.read_through_sequence END AS is_read,
                COALESCE(es.is_starred, FALSE) AS is_starred
         FROM subscriptions s
         JOIN feeds f ON f.id = s.feed_id
         JOIN entries e ON e.feed_id = s.feed_id AND e.feed_sequence > s.start_sequence
         {state_join}
         WHERE s.user_id = {user} AND e.ingest_generation <= {snapshot}"
    );
    if let Some(feed_id) = query.feed_id.as_deref() {
        let feed = sql.bind(feed_id);
        text.push_str(&format!(" AND s.feed_id = {feed}"));
    }
    if let Some(category_id) = query.category_id.as_deref() {
        let category = sql.bind(category_id);
        text.push_str(&format!(" AND s.category_id = {category}"));
    }
    match query.state {
        EntryListState::All => {}
        EntryListState::Unread => text.push_str(
            " AND (es.read_override = FALSE
                    OR (es.read_override IS NULL
                        AND e.feed_sequence > s.read_through_sequence))",
        ),
        EntryListState::Starred => {}
    }
    if let Some(cursor) = cursor {
        let sort_before = sql.bind(cursor.sort_at_us);
        let sort_tie = sql.bind(cursor.sort_at_us);
        let entry_before = sql.bind(cursor.entry_id.as_str());
        text.push_str(&format!(
            " AND (e.sort_at_us < {sort_before}
                   OR (e.sort_at_us = {sort_tie} AND e.id < {entry_before}))"
        ));
    }
    let limit = sql.bind(i64::from(query.limit) + 1);
    text.push_str(&format!(
        " ORDER BY e.sort_at_us DESC, e.id DESC LIMIT {limit}"
    ));
    sql.finish(text)
}

fn detail_statement(backend: DbBackend, user_id: &str, entry_id: &str) -> Statement {
    let mut sql = Sql::new(backend);
    let user = sql.bind(user_id);
    let entry = sql.bind(entry_id);
    sql.finish(format!(
        "SELECT e.id AS entry_id, e.feed_id AS feed_id, f.title AS feed_title,
                f.site_url AS site_url, f.normalized_url AS feed_url, e.title AS entry_title,
                e.author AS author, e.summary AS summary, e.canonical_url AS canonical_url,
                e.published_at_us AS published_at_us, e.sort_at_us AS sort_at_us,
                CASE WHEN es.read_override IS NOT NULL THEN es.read_override
                     ELSE e.feed_sequence <= s.read_through_sequence END AS is_read,
                COALESCE(es.is_starred, FALSE) AS is_starred,
                e.sanitized_content AS sanitized_content, e.enclosure_json AS enclosure_json
         FROM subscriptions s
         JOIN feeds f ON f.id = s.feed_id
         JOIN entries e ON e.feed_id = s.feed_id AND e.feed_sequence > s.start_sequence
         LEFT JOIN entry_states es ON es.user_id = s.user_id AND es.entry_id = e.id
                                  AND es.feed_id = e.feed_id
                                  AND es.feed_sequence = e.feed_sequence
         WHERE s.user_id = {user} AND e.id = {entry}
         LIMIT 1"
    ))
}

fn decode_list_item(row: QueryResult) -> Result<EntryListItemDto, RepositoryError> {
    let feed_url: String = required(&row, "feed_url")?;
    Ok(EntryListItemDto {
        entry_id: required(&row, "entry_id")?,
        feed_id: required(&row, "feed_id")?,
        feed_title: effective_feed_title(optional(&row, "feed_title")?, &feed_url)?,
        site_url: optional(&row, "site_url")?,
        title: optional(&row, "entry_title")?,
        author: optional(&row, "author")?,
        summary: optional(&row, "summary")?,
        canonical_url: optional(&row, "canonical_url")?,
        published_at_us: optional(&row, "published_at_us")?,
        sort_at_us: required(&row, "sort_at_us")?,
        is_read: required(&row, "is_read")?,
        is_starred: required(&row, "is_starred")?,
    })
}

fn decode_detail(row: QueryResult) -> Result<EntryDetailDto, RepositoryError> {
    let feed_url: String = required(&row, "feed_url")?;
    let storage: String = required(&row, "sanitized_content")?;
    let content = EntryContentDetail::decode(&storage).map_err(RepositoryError::Content)?;
    let enclosures = optional::<String>(&row, "enclosure_json")?
        .as_deref()
        .map(decode_enclosures)
        .transpose()?;
    Ok(EntryDetailDto {
        entry_id: required(&row, "entry_id")?,
        feed_id: required(&row, "feed_id")?,
        feed_title: effective_feed_title(optional(&row, "feed_title")?, &feed_url)?,
        site_url: optional(&row, "site_url")?,
        title: optional(&row, "entry_title")?,
        author: optional(&row, "author")?,
        summary: optional(&row, "summary")?,
        canonical_url: optional(&row, "canonical_url")?,
        published_at_us: optional(&row, "published_at_us")?,
        sort_at_us: required(&row, "sort_at_us")?,
        is_read: required(&row, "is_read")?,
        is_starred: required(&row, "is_starred")?,
        content_html: content.html().to_owned(),
        inert_images: content
            .inert_images()
            .iter()
            .map(|image| InertImageDto {
                image_index: image.image_index(),
                source_url: image.source_url().to_owned(),
                alt: image.alt().map(str::to_owned),
                width: image.width(),
                height: image.height(),
            })
            .collect(),
        enclosures,
    })
}

fn decode_enclosures(storage: &str) -> Result<Vec<EnclosureDto>, RepositoryError> {
    let envelope: EnclosureEnvelope =
        serde_json::from_str(storage).map_err(|_| RepositoryError::CorruptData)?;
    if envelope.version != 1
        || serde_json::to_string(&envelope).map_err(|_| RepositoryError::CorruptData)? != storage
    {
        return Err(RepositoryError::CorruptData);
    }
    Ok(envelope.items)
}

fn effective_feed_title(title: Option<String>, feed_url: &str) -> Result<String, RepositoryError> {
    if let Some(title) = title.filter(|title| !title.trim().is_empty()) {
        return Ok(title);
    }
    url::Url::parse(feed_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .filter(|host| !host.is_empty())
        .ok_or(RepositoryError::CorruptData)
}

fn filter_hash(
    user_id: &str,
    state: EntryListState,
    feed_id: Option<&str>,
    category_id: Option<&str>,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"raindrop-entry-filter\0v1");
    hash_frame(&mut hasher, user_id.as_bytes());
    hash_frame(&mut hasher, state.as_str().as_bytes());
    hash_frame(&mut hasher, feed_id.unwrap_or("").as_bytes());
    hash_frame(&mut hasher, category_id.unwrap_or("").as_bytes());
    hash_frame(
        &mut hasher,
        b"order=sort_at_us-desc,entry_id-desc;snapshot=ingest_generation",
    );
    hasher.finalize().to_hex().to_string()
}

fn hash_frame(hasher: &mut blake3::Hasher, value: &[u8]) {
    let length = u32::try_from(value.len()).expect("validated filter fields fit in u32");
    hasher.update(&length.to_be_bytes());
    hasher.update(value);
}

fn encode_cursor(cursor: &CursorPayload) -> Result<String, RepositoryError> {
    let json = serde_json::to_vec(cursor).map_err(|_| RepositoryError::CorruptData)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
}

fn decode_cursor(encoded: &str) -> Result<CursorPayload, RepositoryError> {
    if encoded.is_empty()
        || encoded.len() > MAX_CURSOR_BYTES
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(RepositoryError::InvalidCursor);
    }
    let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| RepositoryError::InvalidCursor)?;
    if json.len() > 768 {
        return Err(RepositoryError::InvalidCursor);
    }
    let cursor: CursorPayload =
        serde_json::from_slice(&json).map_err(|_| RepositoryError::InvalidCursor)?;
    if cursor.v != CURSOR_VERSION
        || cursor.snapshot_generation < 0
        || cursor.filter_hash.len() != 64
        || !cursor
            .filter_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || validate_uuid(&cursor.entry_id).is_err()
        || serde_json::to_vec(&cursor).map_err(|_| RepositoryError::InvalidCursor)? != json
        || base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&json) != encoded
    {
        return Err(RepositoryError::InvalidCursor);
    }
    Ok(cursor)
}

fn required<T>(row: &QueryResult, column: &str) -> Result<T, RepositoryError>
where
    T: sea_orm::TryGetable,
{
    row.try_get("", column)
        .map_err(|_| RepositoryError::CorruptData)
}

fn optional<T>(row: &QueryResult, column: &str) -> Result<Option<T>, RepositoryError>
where
    T: sea_orm::TryGetable,
{
    row.try_get("", column)
        .map_err(|_| RepositoryError::CorruptData)
}

struct Sql {
    backend: DbBackend,
    values: Vec<Value>,
}

impl Sql {
    fn new(backend: DbBackend) -> Self {
        Self {
            backend,
            values: Vec::new(),
        }
    }

    fn bind(&mut self, value: impl Into<Value>) -> String {
        self.values.push(value.into());
        if self.backend == DatabaseBackend::Postgres {
            format!("${}", self.values.len())
        } else {
            "?".to_owned()
        }
    }

    fn finish(self, sql: String) -> Statement {
        Statement::from_sql_and_values(self.backend, sql, self.values)
    }
}

fn explain_statement(statement: Statement) -> Statement {
    let prefix = match statement.db_backend {
        DatabaseBackend::Sqlite => "EXPLAIN QUERY PLAN ",
        DatabaseBackend::Postgres => "EXPLAIN (FORMAT TEXT) ",
        DatabaseBackend::MySql => "EXPLAIN ",
    };
    Statement::from_sql_and_values(
        statement.db_backend,
        format!("{prefix}{}", statement.sql),
        statement.values.map(|values| values.0).unwrap_or_default(),
    )
}

fn explain_line(backend: DbBackend, row: &QueryResult) -> Result<String, RepositoryError> {
    match backend {
        DatabaseBackend::Sqlite => required(row, "detail"),
        DatabaseBackend::Postgres => required(row, "QUERY PLAN"),
        DatabaseBackend::MySql => {
            let table: Option<String> = optional(row, "table")?;
            let key: Option<String> = optional(row, "key")?;
            let access: Option<String> = optional(row, "type")?;
            let extra: Option<String> = optional(row, "Extra")?;
            Ok(format!(
                "table={} key={} type={} extra={}",
                table.as_deref().unwrap_or(""),
                key.as_deref().unwrap_or(""),
                access.as_deref().unwrap_or(""),
                extra.as_deref().unwrap_or("")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_rejects_noncanonical_and_unknown_fields() {
        let payload = CursorPayload {
            v: 1,
            filter_hash: "a".repeat(64),
            snapshot_generation: 1,
            sort_at_us: 2,
            entry_id: "00000000-0000-4000-8000-000000000001".to_owned(),
        };
        let canonical = encode_cursor(&payload).unwrap();
        assert_eq!(decode_cursor(&canonical).unwrap(), payload);

        let unknown = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            br#"{"v":1,"filterHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","snapshotGeneration":1,"sortAtUs":2,"entryId":"00000000-0000-4000-8000-000000000001","extra":true}"#,
        );
        assert!(matches!(
            decode_cursor(&unknown),
            Err(RepositoryError::InvalidCursor)
        ));
        assert!(matches!(
            decode_cursor(&(canonical + "=")),
            Err(RepositoryError::InvalidCursor)
        ));
    }
}
