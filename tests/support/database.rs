use raindrop::db::{
    DatabaseConfig, connect,
    entities::{entry, entry_state, feed, subscription, user},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection};
use secrecy::SecretString;
use time::{OffsetDateTime, macros::datetime};

pub const USER_A_ID: &str = "00000000-0000-4000-8000-000000000001";
pub const USER_B_ID: &str = "00000000-0000-4000-8000-000000000002";
pub const FEED_ID: &str = "00000000-0000-4000-8000-000000000101";
pub const SUBSCRIPTION_A_ID: &str = "00000000-0000-4000-8000-000000000201";
pub const SUBSCRIPTION_B_ID: &str = "00000000-0000-4000-8000-000000000202";
pub const ENTRY_A_ID: &str = "00000000-0000-4000-8000-000000000301";
pub const ENTRY_B_ID: &str = "00000000-0000-4000-8000-000000000302";
pub const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
pub const HASH_D: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const IDENTITY_AT: OffsetDateTime = datetime!(2026-07-16 12:00:00 UTC);

pub async fn connect_for_contract(database_url: SecretString) -> DatabaseConnection {
    connect(&DatabaseConfig::new(database_url))
        .await
        .unwrap_or_else(|_| panic!("RSS contract database should connect"))
}

pub async fn insert_user(database: &DatabaseConnection, id: &str, username: &str) {
    user::ActiveModel {
        id: Set(id.to_owned()),
        username: Set(username.to_owned()),
        normalized_username: Set(username.to_owned()),
        email: Set(None),
        password_hash: Set("test-hash".to_owned()),
        is_disabled: Set(false),
        created_at: Set(IDENTITY_AT),
        last_login_at: Set(None),
    }
    .insert(database)
    .await
    .expect("user should insert");
}

pub async fn insert_feed(database: &DatabaseConnection, at: OffsetDateTime) {
    feed::ActiveModel {
        id: Set(FEED_ID.to_owned()),
        source_url: Set("https://example.com/feed.xml".to_owned()),
        normalized_url: Set("https://example.com/feed.xml".to_owned()),
        normalized_url_hash: Set(HASH_A.to_owned()),
        fetch_url: Set("https://cdn.example.com/feed.xml".to_owned()),
        title: Set(None),
        site_url: Set(None),
        validator_url: Set(Some("https://cdn.example.com/feed.xml".to_owned())),
        etag: Set(Some("\"feed-v1\"".to_owned())),
        last_modified: Set(Some("Thu, 16 Jul 2026 12:00:00 GMT".to_owned())),
        response_content_hash: Set(Some(HASH_B.to_owned())),
        entry_sequence_head: Set(2),
        last_attempt_at: Set(Some(at)),
        last_success_at: Set(Some(at)),
        last_changed_at: Set(Some(at)),
        next_fetch_at: Set(at + time::Duration::minutes(5)),
        retry_after_at: Set(Some(at + time::Duration::minutes(10))),
        consecutive_failures: Set(0),
        last_error_code: Set(None),
        is_disabled: Set(false),
        orphaned_at: Set(Some(at)),
        lease_owner: Set(Some("worker-a".to_owned())),
        lease_token: Set(1),
        lease_until: Set(Some(at + time::Duration::minutes(1))),
        created_at: Set(at),
        updated_at: Set(at),
    }
    .insert(database)
    .await
    .expect("feed should insert");
}

#[must_use]
pub fn subscription_model(
    id: &str,
    user_id: &str,
    at: OffsetDateTime,
) -> subscription::ActiveModel {
    subscription::ActiveModel {
        id: Set(id.to_owned()),
        user_id: Set(user_id.to_owned()),
        feed_id: Set(FEED_ID.to_owned()),
        title_override: Set(None),
        position: Set(0),
        start_sequence: Set(1),
        read_through_sequence: Set(0),
        state_revision: Set(0),
        created_at: Set(at),
        updated_at: Set(at),
    }
}

pub async fn insert_subscription(
    database: &DatabaseConnection,
    id: &str,
    user_id: &str,
    at: OffsetDateTime,
) {
    subscription_model(id, user_id, at)
        .insert(database)
        .await
        .expect("subscription should insert");
}

#[must_use]
pub fn entry_model(
    id: &str,
    feed_sequence: i64,
    identity: &str,
    identity_hash: &str,
    published_at_us: Option<i64>,
    at: OffsetDateTime,
) -> entry::ActiveModel {
    entry::ActiveModel {
        id: Set(id.to_owned()),
        feed_id: Set(FEED_ID.to_owned()),
        feed_sequence: Set(feed_sequence),
        ingest_generation: Set(1),
        identity_kind: Set("GUID".to_owned()),
        identity: Set(identity.to_owned()),
        identity_hash: Set(identity_hash.to_owned()),
        canonical_url: Set(Some(format!(
            "https://example.com/articles/{feed_sequence}"
        ))),
        title: Set(Some(format!("Entry {feed_sequence}"))),
        author: Set(Some("Example Author".to_owned())),
        sanitized_content: Set(
            "rdsc:v1:{\"html\":\"<p>Safe content</p>\",\"inertImages\":[]}".to_owned(),
        ),
        summary: Set(Some("Safe summary".to_owned())),
        published_at_us: Set(published_at_us),
        sort_at_us: Set(2_147_483_648_000_001 + feed_sequence),
        inserted_at: Set(at),
        updated_at: Set(at),
        source_content_hash: Set(HASH_D.to_owned()),
        content_hash: Set(HASH_D.to_owned()),
        pipeline_version: Set("sanitize-v1".to_owned()),
        direction: Set(Some("LTR".to_owned())),
        enclosure_json: Set(Some("{\"version\":1,\"items\":[]}".to_owned())),
    }
}

pub async fn insert_entry(
    database: &DatabaseConnection,
    id: &str,
    feed_sequence: i64,
    identity: &str,
    identity_hash: &str,
    published_at_us: Option<i64>,
    at: OffsetDateTime,
) {
    entry_model(
        id,
        feed_sequence,
        identity,
        identity_hash,
        published_at_us,
        at,
    )
    .insert(database)
    .await
    .expect("entry should insert");
}

pub async fn insert_entry_state(
    database: &DatabaseConnection,
    user_id: &str,
    feed_sequence: i64,
    at: OffsetDateTime,
) -> Result<entry_state::Model, sea_orm::DbErr> {
    entry_state::ActiveModel {
        user_id: Set(user_id.to_owned()),
        entry_id: Set(ENTRY_A_ID.to_owned()),
        feed_id: Set(FEED_ID.to_owned()),
        feed_sequence: Set(feed_sequence),
        read_override: Set(Some(true)),
        is_starred: Set(true),
        starred_at: Set(Some(at)),
        revision: Set(1),
        updated_at: Set(at),
    }
    .insert(database)
    .await
}
