use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use quick_xml::{
    Reader,
    encoding::Decoder,
    events::{BytesStart, Event},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend,
    DatabaseTransaction, DbBackend, DbErr, EntityTrait, QueryFilter, QueryOrder, Statement,
    TransactionTrait,
};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::entities::{category, feed, subscription};

use super::{FeedRepository, FeedUrlPolicy, NormalizedFeedUrl, lifecycle::is_unique_violation};

pub const MAX_OPML_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_OPML_OUTLINES: usize = 10_000;

const MAX_XML_DEPTH: usize = 128;
const MAX_XML_EVENTS: usize = 100_000;
const MAX_ATTRIBUTES_PER_ELEMENT: usize = 64;
const MAX_ATTRIBUTE_BYTES: usize = 64 * 1024;
const MAX_SUBSCRIPTIONS_PER_USER: usize = 1_000;
const MAX_CATEGORIES_PER_USER: usize = 250;
const POSITION_STEP: i64 = 1_024;
const INITIAL_VISIBLE_ENTRY_COUNT: i64 = 100;

#[derive(Clone)]
pub struct OpmlDocument {
    feeds: Vec<OpmlFeed>,
    outline_count: usize,
    invalid_count: usize,
    document_duplicate_count: usize,
}

impl fmt::Debug for OpmlDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpmlDocument")
            .field("feed_count", &self.feeds.len())
            .field("outline_count", &self.outline_count)
            .field("invalid_count", &self.invalid_count)
            .field("document_duplicate_count", &self.document_duplicate_count)
            .finish()
    }
}

impl OpmlDocument {
    pub fn parse(input: &[u8]) -> Result<Self, OpmlError> {
        if input.is_empty() {
            return Err(OpmlError::Malformed);
        }
        if input.len() > MAX_OPML_BYTES {
            return Err(OpmlError::DocumentTooLarge);
        }

        let mut reader = Reader::from_reader(input);
        reader.config_mut().enable_all_checks(true);
        let mut depth = 0_usize;
        let mut events = 0_usize;
        let mut root_seen = false;
        let mut root_closed = false;
        let mut body_seen = false;
        let mut body_depth = None;
        let mut outline_count = 0_usize;
        let mut invalid_count = 0_usize;
        let mut document_duplicate_count = 0_usize;
        let mut categories = Vec::<CategoryFrame>::new();
        let mut feeds = Vec::new();
        let mut normalized_urls = HashSet::new();

        loop {
            let event = reader.read_event().map_err(|_| OpmlError::Malformed)?;
            if !matches!(event, Event::Eof) {
                events = events.checked_add(1).ok_or(OpmlError::EventLimit)?;
                if events > MAX_XML_EVENTS {
                    return Err(OpmlError::EventLimit);
                }
            }

            match event {
                Event::Start(element) => {
                    let name = element_name(&element)?;
                    if depth == 0 {
                        if root_seen || root_closed || name != "opml" {
                            return Err(OpmlError::Malformed);
                        }
                        root_seen = true;
                    }
                    let attributes = collect_attributes(&element, reader.decoder())?;
                    let open_depth = depth.checked_add(1).ok_or(OpmlError::DepthLimit)?;
                    if open_depth > MAX_XML_DEPTH {
                        return Err(OpmlError::DepthLimit);
                    }
                    if name == "body" && depth == 1 {
                        if body_seen {
                            return Err(OpmlError::Malformed);
                        }
                        body_seen = true;
                        body_depth = Some(open_depth);
                    }
                    if name == "outline" {
                        outline_count = count_outline(outline_count)?;
                        if body_depth.is_some_and(|body| open_depth > body) {
                            match parse_outline(&attributes, categories.last()) {
                                ParsedOutline::Feed(candidate) => {
                                    if normalized_urls
                                        .insert(candidate.normalized.complete().to_owned())
                                    {
                                        feeds.push(candidate);
                                    } else {
                                        document_duplicate_count += 1;
                                    }
                                }
                                ParsedOutline::Category(category) => {
                                    categories.push(CategoryFrame {
                                        depth: open_depth,
                                        category,
                                    })
                                }
                                ParsedOutline::InvalidFeed => invalid_count += 1,
                            }
                        }
                    }
                    depth = open_depth;
                }
                Event::Empty(element) => {
                    let name = element_name(&element)?;
                    if depth == 0 {
                        if root_seen || root_closed || name != "opml" {
                            return Err(OpmlError::Malformed);
                        }
                        root_seen = true;
                        root_closed = true;
                    }
                    let attributes = collect_attributes(&element, reader.decoder())?;
                    let element_depth = depth.checked_add(1).ok_or(OpmlError::DepthLimit)?;
                    if element_depth > MAX_XML_DEPTH {
                        return Err(OpmlError::DepthLimit);
                    }
                    if name == "body" && depth == 1 {
                        if body_seen {
                            return Err(OpmlError::Malformed);
                        }
                        body_seen = true;
                    }
                    if name == "outline" {
                        outline_count = count_outline(outline_count)?;
                        if body_depth.is_some_and(|body| element_depth > body) {
                            match parse_outline(&attributes, categories.last()) {
                                ParsedOutline::Feed(candidate) => {
                                    if normalized_urls
                                        .insert(candidate.normalized.complete().to_owned())
                                    {
                                        feeds.push(candidate);
                                    } else {
                                        document_duplicate_count += 1;
                                    }
                                }
                                ParsedOutline::Category(_) => {}
                                ParsedOutline::InvalidFeed => invalid_count += 1,
                            }
                        }
                    }
                }
                Event::End(element) => {
                    if depth == 0 {
                        return Err(OpmlError::Malformed);
                    }
                    let qualified_name = element.name();
                    let name = decode_name(qualified_name.as_ref())?;
                    if name == "outline"
                        && categories.last().is_some_and(|frame| frame.depth == depth)
                    {
                        let _ = categories.pop();
                    }
                    if name == "body" && body_depth == Some(depth) {
                        body_depth = None;
                    }
                    depth -= 1;
                    if depth == 0 {
                        root_closed = true;
                    }
                }
                Event::Text(text) => {
                    let bytes: &[u8] = text.as_ref();
                    if depth == 0 && !bytes.iter().all(u8::is_ascii_whitespace) {
                        return Err(OpmlError::Malformed);
                    }
                }
                Event::CData(_) | Event::GeneralRef(_) | Event::DocType(_) => {
                    return Err(OpmlError::ForbiddenXmlConstruct);
                }
                Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
                Event::Eof => break,
            }
        }

        if !root_seen || !root_closed || depth != 0 || !body_seen {
            return Err(OpmlError::Malformed);
        }

        Ok(Self {
            feeds,
            outline_count,
            invalid_count,
            document_duplicate_count,
        })
    }

    #[must_use]
    pub const fn outline_count(&self) -> usize {
        self.outline_count
    }

    #[must_use]
    pub fn valid_count(&self) -> usize {
        self.feeds.len()
    }

    #[must_use]
    pub const fn invalid_count(&self) -> usize {
        self.invalid_count
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpmlPreview {
    pub outline_count: usize,
    pub valid_count: usize,
    pub new_count: usize,
    pub duplicate_count: usize,
    pub invalid_count: usize,
    pub category_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpmlImportResult {
    pub outline_count: usize,
    pub valid_count: usize,
    pub imported_count: usize,
    pub duplicate_count: usize,
    pub invalid_count: usize,
    pub created_category_count: usize,
}

#[derive(thiserror::Error)]
pub enum OpmlError {
    #[error("OPML document exceeds the byte limit")]
    DocumentTooLarge,
    #[error("OPML document exceeds the outline limit")]
    OutlineLimit,
    #[error("OPML document exceeds the XML depth limit")]
    DepthLimit,
    #[error("OPML document exceeds the XML event limit")]
    EventLimit,
    #[error("OPML document contains a forbidden XML construct")]
    ForbiddenXmlConstruct,
    #[error("OPML document is malformed")]
    Malformed,
    #[error("user identifier is invalid")]
    InvalidUserId,
    #[error("subscription limit reached")]
    SubscriptionLimit,
    #[error("category limit reached")]
    CategoryLimit,
    #[error("stored feed identity conflicts with the imported URL")]
    IdentityCollision,
    #[error("stored OPML data is corrupt")]
    CorruptData,
    #[error("OPML database operation failed")]
    Database(#[source] DbErr),
}

impl fmt::Debug for OpmlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DocumentTooLarge => "OpmlError::DocumentTooLarge",
            Self::OutlineLimit => "OpmlError::OutlineLimit",
            Self::DepthLimit => "OpmlError::DepthLimit",
            Self::EventLimit => "OpmlError::EventLimit",
            Self::ForbiddenXmlConstruct => "OpmlError::ForbiddenXmlConstruct",
            Self::Malformed => "OpmlError::Malformed",
            Self::InvalidUserId => "OpmlError::InvalidUserId",
            Self::SubscriptionLimit => "OpmlError::SubscriptionLimit",
            Self::CategoryLimit => "OpmlError::CategoryLimit",
            Self::IdentityCollision => "OpmlError::IdentityCollision",
            Self::CorruptData => "OpmlError::CorruptData",
            Self::Database(_) => "OpmlError::Database([REDACTED])",
        })
    }
}

impl From<DbErr> for OpmlError {
    fn from(error: DbErr) -> Self {
        Self::Database(error)
    }
}

#[derive(Clone)]
struct OpmlFeed {
    source_url: String,
    normalized: NormalizedFeedUrl,
    title: Option<String>,
    category: Option<ImportCategory>,
}

#[derive(Clone)]
struct ImportCategory {
    display: String,
    normalized: String,
}

struct CategoryFrame {
    depth: usize,
    category: Option<ImportCategory>,
}

enum ParsedOutline {
    Feed(OpmlFeed),
    Category(Option<ImportCategory>),
    InvalidFeed,
}

struct ElementAttributes(Vec<(String, String)>);

impl ElementAttributes {
    fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

impl FeedRepository {
    pub async fn preview_opml(
        &self,
        user_id: &str,
        document: &OpmlDocument,
    ) -> Result<OpmlPreview, OpmlError> {
        validate_user_id(user_id)?;
        let existing = existing_normalized_urls(self.connection(), user_id).await?;
        let database_duplicates = document
            .feeds
            .iter()
            .filter(|candidate| existing.contains(candidate.normalized.complete()))
            .count();
        let category_count = document
            .feeds
            .iter()
            .filter_map(|candidate| candidate.category.as_ref())
            .map(|category| category.normalized.as_str())
            .collect::<HashSet<_>>()
            .len();
        Ok(OpmlPreview {
            outline_count: document.outline_count,
            valid_count: document.feeds.len(),
            new_count: document.feeds.len().saturating_sub(database_duplicates),
            duplicate_count: document.document_duplicate_count + database_duplicates,
            invalid_count: document.invalid_count,
            category_count,
        })
    }

    pub async fn import_opml(
        &self,
        user_id: &str,
        document: &OpmlDocument,
    ) -> Result<OpmlImportResult, OpmlError> {
        validate_user_id(user_id)?;
        for attempt in 0..3 {
            match self.import_opml_once(user_id, document).await {
                Err(OpmlError::Database(error)) if attempt < 2 && is_unique_violation(&error) => {}
                result => return result,
            }
        }
        Err(OpmlError::CorruptData)
    }

    async fn import_opml_once(
        &self,
        user_id: &str,
        document: &OpmlDocument,
    ) -> Result<OpmlImportResult, OpmlError> {
        let transaction = self.connection().begin().await?;
        let result = import_document(&transaction, user_id, document).await;
        match result {
            Ok(imported) => {
                transaction.commit().await?;
                Ok(imported)
            }
            Err(error) => {
                transaction.rollback().await?;
                Err(error)
            }
        }
    }

    pub async fn export_opml(&self, user_id: &str) -> Result<Vec<u8>, OpmlError> {
        validate_user_id(user_id)?;
        let categories = category::Entity::find()
            .filter(category::Column::UserId.eq(user_id))
            .order_by_asc(category::Column::Position)
            .order_by_asc(category::Column::Id)
            .all(self.connection())
            .await?;
        let subscriptions = subscription::Entity::find()
            .filter(subscription::Column::UserId.eq(user_id))
            .order_by_asc(subscription::Column::Position)
            .order_by_asc(subscription::Column::Id)
            .all(self.connection())
            .await?;
        let feeds = load_feeds_for_subscriptions(self.connection(), &subscriptions).await?;
        render_opml(categories, subscriptions, feeds)
    }
}

async fn import_document(
    transaction: &DatabaseTransaction,
    user_id: &str,
    document: &OpmlDocument,
) -> Result<OpmlImportResult, OpmlError> {
    let backend = transaction.get_database_backend();
    lock_active_user(transaction, backend, user_id).await?;

    let stored_categories = category::Entity::find()
        .filter(category::Column::UserId.eq(user_id))
        .all(transaction)
        .await?;
    if stored_categories.len() > MAX_CATEGORIES_PER_USER {
        return Err(OpmlError::CorruptData);
    }
    let mut categories_by_title = stored_categories
        .iter()
        .map(|stored| (stored.normalized_title.clone(), stored.id.clone()))
        .collect::<HashMap<_, _>>();
    let mut next_category_position = stored_categories
        .iter()
        .map(|stored| stored.position)
        .max()
        .unwrap_or(0);
    if next_category_position < 0 {
        return Err(OpmlError::CorruptData);
    }

    let stored_subscriptions = subscription::Entity::find()
        .filter(subscription::Column::UserId.eq(user_id))
        .all(transaction)
        .await?;
    if stored_subscriptions.len() > MAX_SUBSCRIPTIONS_PER_USER {
        return Err(OpmlError::CorruptData);
    }
    let mut subscribed_feed_ids = stored_subscriptions
        .iter()
        .map(|stored| stored.feed_id.clone())
        .collect::<HashSet<_>>();
    let mut next_subscription_position = stored_subscriptions
        .iter()
        .map(|stored| stored.position)
        .max()
        .unwrap_or(0);
    if next_subscription_position < 0 {
        return Err(OpmlError::CorruptData);
    }

    let hashes = document
        .feeds
        .iter()
        .map(|candidate| candidate.normalized.url_hash().to_owned())
        .collect::<Vec<_>>();
    let mut feeds_by_hash = load_feeds_by_hash(transaction, &hashes).await?;
    let mut imported_count = 0_usize;
    let mut database_duplicate_count = 0_usize;
    let mut created_category_count = 0_usize;

    for candidate in &document.feeds {
        let stored_feed = if let Some(stored) = feeds_by_hash.get(candidate.normalized.url_hash()) {
            if stored.normalized_url != candidate.normalized.complete() {
                return Err(OpmlError::IdentityCollision);
            }
            stored.clone()
        } else {
            let now = OffsetDateTime::now_utc();
            let stored = feed::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                source_url: Set(candidate.source_url.clone()),
                normalized_url: Set(candidate.normalized.complete().to_owned()),
                normalized_url_hash: Set(candidate.normalized.url_hash().to_owned()),
                fetch_url: Set(candidate.normalized.complete().to_owned()),
                title: Set(None),
                site_url: Set(None),
                validator_url: Set(None),
                etag: Set(None),
                last_modified: Set(None),
                response_content_hash: Set(None),
                entry_sequence_head: Set(0),
                last_attempt_at: Set(None),
                last_success_at: Set(None),
                last_changed_at: Set(None),
                next_fetch_at: Set(now),
                retry_after_at: Set(None),
                consecutive_failures: Set(0),
                last_error_code: Set(None),
                is_disabled: Set(false),
                orphaned_at: Set(None),
                lease_owner: Set(None),
                lease_token: Set(0),
                lease_until: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(transaction)
            .await?;
            feeds_by_hash.insert(candidate.normalized.url_hash().to_owned(), stored.clone());
            stored
        };

        if stored_feed.is_disabled {
            return Err(OpmlError::CorruptData);
        }
        if subscribed_feed_ids.contains(&stored_feed.id) {
            database_duplicate_count += 1;
            continue;
        }
        if subscribed_feed_ids.len() >= MAX_SUBSCRIPTIONS_PER_USER {
            return Err(OpmlError::SubscriptionLimit);
        }

        if stored_feed.orphaned_at.is_some() {
            let mut active: feed::ActiveModel = stored_feed.clone().into();
            active.orphaned_at = Set(None);
            active.updated_at = Set(OffsetDateTime::now_utc());
            active.update(transaction).await?;
        }

        let category_id = if let Some(imported_category) = &candidate.category {
            if let Some(category_id) = categories_by_title.get(&imported_category.normalized) {
                Some(category_id.clone())
            } else {
                if categories_by_title.len() >= MAX_CATEGORIES_PER_USER {
                    return Err(OpmlError::CategoryLimit);
                }
                next_category_position = next_category_position
                    .checked_add(POSITION_STEP)
                    .ok_or(OpmlError::CorruptData)?;
                let now = OffsetDateTime::now_utc();
                let inserted = category::ActiveModel {
                    id: Set(Uuid::new_v4().to_string()),
                    user_id: Set(user_id.to_owned()),
                    title: Set(imported_category.display.clone()),
                    normalized_title: Set(imported_category.normalized.clone()),
                    position: Set(next_category_position),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(transaction)
                .await?;
                categories_by_title
                    .insert(imported_category.normalized.clone(), inserted.id.clone());
                created_category_count += 1;
                Some(inserted.id)
            }
        } else {
            None
        };

        next_subscription_position = next_subscription_position
            .checked_add(POSITION_STEP)
            .ok_or(OpmlError::CorruptData)?;
        let initial_frontier = stored_feed
            .entry_sequence_head
            .saturating_sub(INITIAL_VISIBLE_ENTRY_COUNT)
            .max(0);
        let now = OffsetDateTime::now_utc();
        subscription::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            user_id: Set(user_id.to_owned()),
            feed_id: Set(stored_feed.id.clone()),
            category_id: Set(category_id),
            title_override: Set(candidate.title.clone()),
            position: Set(next_subscription_position),
            start_sequence: Set(initial_frontier),
            read_through_sequence: Set(initial_frontier),
            state_revision: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(transaction)
        .await?;
        subscribed_feed_ids.insert(stored_feed.id);
        imported_count += 1;
    }

    Ok(OpmlImportResult {
        outline_count: document.outline_count,
        valid_count: document.feeds.len(),
        imported_count,
        duplicate_count: document.document_duplicate_count + database_duplicate_count,
        invalid_count: document.invalid_count,
        created_category_count,
    })
}

fn parse_outline(attributes: &ElementAttributes, parent: Option<&CategoryFrame>) -> ParsedOutline {
    let label = attributes.get("title").or_else(|| attributes.get("text"));
    let Some(source_url) = attributes.get("xmlUrl") else {
        return ParsedOutline::Category(label.and_then(normalize_category));
    };
    let normalized = match FeedUrlPolicy::new(false).normalize(source_url) {
        Ok(normalized) => normalized,
        Err(_) => return ParsedOutline::InvalidFeed,
    };
    ParsedOutline::Feed(OpmlFeed {
        source_url: source_url.to_owned(),
        normalized,
        title: label.and_then(normalize_feed_title),
        category: parent.and_then(|frame| frame.category.clone()),
    })
}

fn normalize_category(raw: &str) -> Option<ImportCategory> {
    let display = raw.trim();
    if display.is_empty()
        || display.chars().count() > 80
        || display.len() > 200
        || display.chars().any(is_disallowed_control)
    {
        return None;
    }
    let normalized = display.to_lowercase();
    (normalized.len() <= 320).then(|| ImportCategory {
        display: display.to_owned(),
        normalized,
    })
}

fn normalize_feed_title(raw: &str) -> Option<String> {
    let title = raw.trim();
    (!title.is_empty() && title.len() <= 200 && !title.chars().any(is_disallowed_control))
        .then(|| title.to_owned())
}

fn is_disallowed_control(character: char) -> bool {
    matches!(u32::from(character), 0..=31 | 127..=159)
}

fn count_outline(current: usize) -> Result<usize, OpmlError> {
    let next = current.checked_add(1).ok_or(OpmlError::OutlineLimit)?;
    if next > MAX_OPML_OUTLINES {
        Err(OpmlError::OutlineLimit)
    } else {
        Ok(next)
    }
}

fn element_name(element: &BytesStart<'_>) -> Result<String, OpmlError> {
    decode_name(element.name().as_ref()).map(str::to_owned)
}

fn decode_name(value: &[u8]) -> Result<&str, OpmlError> {
    std::str::from_utf8(value).map_err(|_| OpmlError::Malformed)
}

fn collect_attributes(
    element: &BytesStart<'_>,
    decoder: Decoder,
) -> Result<ElementAttributes, OpmlError> {
    let mut values = Vec::new();
    let mut total_bytes = 0_usize;
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| OpmlError::Malformed)?;
        if values.len() == MAX_ATTRIBUTES_PER_ELEMENT {
            return Err(OpmlError::Malformed);
        }
        total_bytes = total_bytes
            .checked_add(attribute.key.as_ref().len())
            .and_then(|total| total.checked_add(attribute.value.len()))
            .ok_or(OpmlError::Malformed)?;
        if total_bytes > MAX_ATTRIBUTE_BYTES {
            return Err(OpmlError::Malformed);
        }
        let key = decode_name(attribute.key.as_ref())?.to_owned();
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
            .map_err(|_| OpmlError::Malformed)?
            .into_owned();
        values.push((key, value));
    }
    Ok(ElementAttributes(values))
}

fn validate_user_id(user_id: &str) -> Result<(), OpmlError> {
    let parsed = Uuid::parse_str(user_id).map_err(|_| OpmlError::InvalidUserId)?;
    if parsed.to_string() == user_id {
        Ok(())
    } else {
        Err(OpmlError::InvalidUserId)
    }
}

async fn existing_normalized_urls<C>(
    connection: &C,
    user_id: &str,
) -> Result<HashSet<String>, OpmlError>
where
    C: ConnectionTrait,
{
    let backend = connection.get_database_backend();
    let sql = if backend == DatabaseBackend::Postgres {
        "SELECT f.normalized_url FROM subscriptions s JOIN feeds f ON f.id = s.feed_id WHERE s.user_id = $1"
    } else {
        "SELECT f.normalized_url FROM subscriptions s JOIN feeds f ON f.id = s.feed_id WHERE s.user_id = ?"
    };
    let rows = connection
        .query_all(Statement::from_sql_and_values(
            backend,
            sql,
            [user_id.into()],
        ))
        .await?;
    rows.into_iter()
        .map(|row| {
            row.try_get("", "normalized_url")
                .map_err(|_| OpmlError::CorruptData)
        })
        .collect()
}

async fn lock_active_user<C>(
    connection: &C,
    backend: DbBackend,
    user_id: &str,
) -> Result<(), OpmlError>
where
    C: ConnectionTrait,
{
    match backend {
        DatabaseBackend::Sqlite => {
            let result = connection
                .execute(Statement::from_sql_and_values(
                    backend,
                    "UPDATE users SET is_disabled = is_disabled WHERE id = ? AND is_disabled = FALSE",
                    [user_id.into()],
                ))
                .await?;
            if result.rows_affected() != 1 {
                return Err(OpmlError::InvalidUserId);
            }
        }
        DatabaseBackend::Postgres | DatabaseBackend::MySql => {
            let sql = if backend == DatabaseBackend::Postgres {
                "SELECT is_disabled FROM users WHERE id = $1 FOR UPDATE"
            } else {
                "SELECT is_disabled FROM users WHERE id = ? FOR UPDATE"
            };
            let row = connection
                .query_one(Statement::from_sql_and_values(
                    backend,
                    sql,
                    [user_id.into()],
                ))
                .await?
                .ok_or(OpmlError::InvalidUserId)?;
            let disabled: bool = row
                .try_get("", "is_disabled")
                .map_err(|_| OpmlError::CorruptData)?;
            if disabled {
                return Err(OpmlError::InvalidUserId);
            }
        }
    }
    Ok(())
}

async fn load_feeds_by_hash<C>(
    connection: &C,
    hashes: &[String],
) -> Result<HashMap<String, feed::Model>, OpmlError>
where
    C: ConnectionTrait,
{
    let mut result = HashMap::new();
    for chunk in hashes.chunks(200) {
        let models = feed::Entity::find()
            .filter(feed::Column::NormalizedUrlHash.is_in(chunk.iter().cloned()))
            .all(connection)
            .await?;
        for model in models {
            if result
                .insert(model.normalized_url_hash.clone(), model)
                .is_some()
            {
                return Err(OpmlError::CorruptData);
            }
        }
    }
    Ok(result)
}

async fn load_feeds_for_subscriptions<C>(
    connection: &C,
    subscriptions: &[subscription::Model],
) -> Result<HashMap<String, feed::Model>, OpmlError>
where
    C: ConnectionTrait,
{
    let ids = subscriptions
        .iter()
        .map(|stored| stored.feed_id.clone())
        .collect::<Vec<_>>();
    let mut feeds = HashMap::new();
    for chunk in ids.chunks(200) {
        for stored in feed::Entity::find()
            .filter(feed::Column::Id.is_in(chunk.iter().cloned()))
            .all(connection)
            .await?
        {
            feeds.insert(stored.id.clone(), stored);
        }
    }
    if feeds.len() != ids.iter().collect::<HashSet<_>>().len() {
        return Err(OpmlError::CorruptData);
    }
    Ok(feeds)
}

fn render_opml(
    categories: Vec<category::Model>,
    subscriptions: Vec<subscription::Model>,
    feeds: HashMap<String, feed::Model>,
) -> Result<Vec<u8>, OpmlError> {
    let mut output = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<opml version=\"2.0\">\n  <head><title>Raindrop subscriptions</title></head>\n  <body>\n",
    );
    let mut by_category = HashMap::<Option<String>, Vec<&subscription::Model>>::new();
    for stored in &subscriptions {
        by_category
            .entry(stored.category_id.clone())
            .or_default()
            .push(stored);
    }

    if let Some(uncategorized) = by_category.remove(&None) {
        for stored in uncategorized {
            render_subscription(&mut output, stored, &feeds, 4)?;
        }
    }
    for category in categories {
        let Some(items) = by_category.remove(&Some(category.id.clone())) else {
            continue;
        };
        output.push_str("    <outline text=\"");
        output.push_str(&escape_xml_attribute(&category.title));
        output.push_str("\" title=\"");
        output.push_str(&escape_xml_attribute(&category.title));
        output.push_str("\">\n");
        for stored in items {
            render_subscription(&mut output, stored, &feeds, 6)?;
        }
        output.push_str("    </outline>\n");
    }
    if !by_category.is_empty() {
        return Err(OpmlError::CorruptData);
    }
    output.push_str("  </body>\n</opml>\n");
    Ok(output.into_bytes())
}

fn render_subscription(
    output: &mut String,
    stored: &subscription::Model,
    feeds: &HashMap<String, feed::Model>,
    indent: usize,
) -> Result<(), OpmlError> {
    let feed = feeds.get(&stored.feed_id).ok_or(OpmlError::CorruptData)?;
    let title = stored
        .title_override
        .as_deref()
        .or(feed.title.as_deref())
        .and_then(bounded_export_title)
        .unwrap_or_else(|| export_fallback_title(&feed.normalized_url));
    output.push_str(&" ".repeat(indent));
    output.push_str("<outline type=\"rss\" text=\"");
    output.push_str(&escape_xml_attribute(&title));
    output.push_str("\" title=\"");
    output.push_str(&escape_xml_attribute(&title));
    output.push_str("\" xmlUrl=\"");
    output.push_str(&escape_xml_attribute(&feed.source_url));
    output.push('"');
    if let Some(site_url) = feed.site_url.as_deref() {
        output.push_str(" htmlUrl=\"");
        output.push_str(&escape_xml_attribute(site_url));
        output.push('"');
    }
    output.push_str("/>\n");
    Ok(())
}

fn bounded_export_title(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut result = String::new();
    for character in trimmed
        .chars()
        .filter(|character| !is_disallowed_control(*character))
    {
        if result.len() + character.len_utf8() > 512 {
            break;
        }
        result.push(character);
    }
    (!result.is_empty()).then_some(result)
}

fn export_fallback_title(normalized_url: &str) -> String {
    url::Url::parse(normalized_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "Feed".to_owned())
}

fn escape_xml_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            character if is_disallowed_control(character) => {}
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_categories_and_counts_invalid_and_duplicate_urls() {
        let document = OpmlDocument::parse(
            br#"<?xml version="1.0"?>
<opml version="2.0"><body>
  <outline text="Tech">
    <outline text="IT Home" xmlUrl="https://www.ithome.com/rss/"/>
    <outline text="Duplicate" xmlUrl="https://www.ithome.com/rss/"/>
  </outline>
  <outline text="Broken" xmlUrl="javascript:alert(1)"/>
</body></opml>"#,
        )
        .expect("OPML should parse");

        assert_eq!(document.outline_count(), 4);
        assert_eq!(document.valid_count(), 1);
        assert_eq!(document.invalid_count(), 1);
        assert_eq!(document.document_duplicate_count, 1);
        assert_eq!(document.feeds[0].title.as_deref(), Some("IT Home"));
        assert_eq!(
            document.feeds[0]
                .category
                .as_ref()
                .map(|category| category.display.as_str()),
            Some("Tech")
        );
    }

    #[test]
    fn rejects_doctype_and_external_entity_constructs() {
        let error = OpmlDocument::parse(
            br#"<!DOCTYPE opml [<!ENTITY xxe SYSTEM "file:///etc/passwd">]><opml version="2.0"><body>&xxe;</body></opml>"#,
        )
        .expect_err("DTD-bearing OPML must fail");
        assert!(matches!(
            error,
            OpmlError::ForbiddenXmlConstruct | OpmlError::Malformed
        ));
    }

    #[test]
    fn escapes_export_attributes() {
        assert_eq!(
            escape_xml_attribute("A&B <\"feed\">'"),
            "A&amp;B &lt;&quot;feed&quot;&gt;&apos;"
        );
    }

    #[test]
    fn rejects_documents_over_the_byte_limit() {
        let input = vec![b' '; MAX_OPML_BYTES + 1];
        assert!(matches!(
            OpmlDocument::parse(&input),
            Err(OpmlError::DocumentTooLarge)
        ));
    }
}
