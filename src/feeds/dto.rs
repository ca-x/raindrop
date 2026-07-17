use std::fmt;

use time::OffsetDateTime;

use super::RefreshStatus;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscribeInput {
    pub url: String,
}

#[derive(Clone, Eq, PartialEq)]
pub struct SubscriptionDto {
    pub subscription_id: String,
    pub feed_id: String,
    pub title: String,
    pub site_url: Option<String>,
    pub start_sequence: i64,
    pub read_through_sequence: i64,
    pub refresh: RefreshDto,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListSubscriptionsQuery {
    pub cursor: Option<String>,
    pub limit: u16,
}

impl Default for ListSubscriptionsQuery {
    fn default() -> Self {
        Self {
            cursor: None,
            limit: 50,
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SubscriptionListItemDto {
    pub subscription_id: String,
    pub feed_id: String,
    pub title: String,
    pub site_url: Option<String>,
    pub unread_count: i64,
    pub refresh: Option<RefreshDto>,
}

impl fmt::Debug for SubscriptionListItemDto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubscriptionListItemDto")
            .field("subscription_id", &self.subscription_id)
            .field("feed_id", &self.feed_id)
            .field("title", &"[REDACTED]")
            .field("site_url", &self.site_url.as_ref().map(|_| "[REDACTED]"))
            .field("unread_count", &self.unread_count)
            .field("refresh", &self.refresh)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionPage {
    pub items: Vec<SubscriptionListItemDto>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscribeOutcome {
    pub created: bool,
    pub subscription: SubscriptionListItemDto,
}

#[derive(Clone, Eq, PartialEq)]
pub struct QueueSubscriptionRefresh {
    pub request_id: String,
}

impl fmt::Debug for QueueSubscriptionRefresh {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QueueSubscriptionRefresh")
            .field("request_id", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for SubscriptionDto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubscriptionDto")
            .field("subscription_id", &self.subscription_id)
            .field("feed_id", &self.feed_id)
            .field("title", &"[REDACTED]")
            .field("site_url", &self.site_url.as_ref().map(|_| "[REDACTED]"))
            .field("start_sequence", &self.start_sequence)
            .field("read_through_sequence", &self.read_through_sequence)
            .field("refresh", &self.refresh)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct RefreshDto {
    pub run_id: String,
    pub status: RefreshStatus,
    pub http_status: Option<i32>,
    pub new_count: i32,
    pub updated_count: i32,
    pub dropped_count: i32,
    pub generation: Option<i64>,
    pub error_code: Option<String>,
    pub retry_at: Option<OffsetDateTime>,
    pub queued_at: OffsetDateTime,
    pub started_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
}

impl fmt::Debug for RefreshDto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RefreshDto")
            .field("run_id", &"[REDACTED]")
            .field("status", &self.status)
            .field("http_status", &self.http_status)
            .field("new_count", &self.new_count)
            .field("updated_count", &self.updated_count)
            .field("dropped_count", &self.dropped_count)
            .field("generation", &self.generation)
            .field("error_code", &self.error_code)
            .field("retry_at", &self.retry_at.map(|_| "[REDACTED]"))
            .field("queued_at", &"[REDACTED]")
            .field("started_at", &self.started_at.map(|_| "[REDACTED]"))
            .field("completed_at", &self.completed_at.map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct EntryListItemDto {
    pub entry_id: String,
    pub feed_id: String,
    pub feed_title: String,
    pub site_url: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub canonical_url: Option<String>,
    pub published_at_us: Option<i64>,
    pub sort_at_us: i64,
    pub is_read: bool,
    pub is_starred: bool,
}

impl fmt::Debug for EntryListItemDto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntryListItemDto")
            .field("entry_id", &self.entry_id)
            .field("feed_id", &self.feed_id)
            .field("feed_title", &"[REDACTED]")
            .field("site_url", &self.site_url.as_ref().map(|_| "[REDACTED]"))
            .field("title", &self.title.as_ref().map(|_| "[REDACTED]"))
            .field("author", &self.author.as_ref().map(|_| "[REDACTED]"))
            .field("summary", &self.summary.as_ref().map(|_| "[REDACTED]"))
            .field(
                "canonical_url",
                &self.canonical_url.as_ref().map(|_| "[REDACTED]"),
            )
            .field("published_at_us", &self.published_at_us)
            .field("sort_at_us", &self.sort_at_us)
            .field("is_read", &self.is_read)
            .field("is_starred", &self.is_starred)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryPage {
    pub items: Vec<EntryListItemDto>,
    pub next_cursor: Option<String>,
    pub snapshot_generation: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UpdateEntryState {
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryStateDto {
    pub entry_id: String,
    pub is_read: bool,
    pub is_starred: bool,
}

#[derive(Clone, Eq, PartialEq)]
pub struct EntryDetailDto {
    pub entry_id: String,
    pub feed_id: String,
    pub feed_title: String,
    pub site_url: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub canonical_url: Option<String>,
    pub published_at_us: Option<i64>,
    pub sort_at_us: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub content_html: String,
    pub inert_images: Vec<InertImageDto>,
    pub enclosures: Option<Vec<EnclosureDto>>,
}

impl fmt::Debug for EntryDetailDto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntryDetailDto")
            .field("entry_id", &self.entry_id)
            .field("feed_id", &self.feed_id)
            .field("feed_title", &"[REDACTED]")
            .field("site_url", &self.site_url.as_ref().map(|_| "[REDACTED]"))
            .field("title", &self.title.as_ref().map(|_| "[REDACTED]"))
            .field("author", &self.author.as_ref().map(|_| "[REDACTED]"))
            .field("summary", &self.summary.as_ref().map(|_| "[REDACTED]"))
            .field(
                "canonical_url",
                &self.canonical_url.as_ref().map(|_| "[REDACTED]"),
            )
            .field("published_at_us", &self.published_at_us)
            .field("sort_at_us", &self.sort_at_us)
            .field("is_read", &self.is_read)
            .field("is_starred", &self.is_starred)
            .field("content_html_bytes", &self.content_html.len())
            .field("inert_image_count", &self.inert_images.len())
            .field(
                "enclosure_count",
                &self.enclosures.as_ref().map(std::vec::Vec::len),
            )
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct InertImageDto {
    pub image_index: u32,
    pub source_url: String,
    pub alt: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl fmt::Debug for InertImageDto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InertImageDto")
            .field("image_index", &self.image_index)
            .field("source_url", &"[REDACTED]")
            .field("alt", &self.alt.as_ref().map(|_| "[REDACTED]"))
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnclosureDto {
    pub url: String,
    pub media_type: Option<String>,
    pub length: Option<String>,
    pub title: Option<String>,
    pub duration: Option<String>,
}

impl fmt::Debug for EnclosureDto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnclosureDto")
            .field("url", &"[REDACTED]")
            .field(
                "media_type",
                &self.media_type.as_ref().map(|_| "[REDACTED]"),
            )
            .field("length", &self.length.as_ref().map(|_| "[REDACTED]"))
            .field("title", &self.title.as_ref().map(|_| "[REDACTED]"))
            .field("duration", &self.duration.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}
