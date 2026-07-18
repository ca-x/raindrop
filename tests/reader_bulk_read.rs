#[allow(dead_code)]
mod support;

use raindrop::{
    db::{
        entities::{category, entry_state, feed, rss_counter, subscription},
        migrate, rollback,
    },
    feeds::{
        EntryListState, FeedRepository, ListEntriesQuery, MarkReadScope, RepositoryError,
        UpdateEntryState,
    },
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter,
};
use secrecy::SecretString;
use tempfile::TempDir;
use time::OffsetDateTime;

use support::database::{
    FEED_ID, HASH_C, USER_A_ID, USER_B_ID, connect_for_contract, entry_model, insert_feed,
    insert_user, subscription_model,
};

const FEED_B_ID: &str = "00000000-0000-4000-8000-000000000102";
const SUBSCRIPTION_A_ID: &str = "00000000-0000-4000-8000-000000000201";
const SUBSCRIPTION_B_ID: &str = "00000000-0000-4000-8000-000000000202";
const CATEGORY_ID: &str = "00000000-0000-4000-8000-000000000501";
const ENTRY_A_ID: &str = "00000000-0000-4000-8000-000000000301";
const ENTRY_B_ID: &str = "00000000-0000-4000-8000-000000000302";
const ENTRY_C_ID: &str = "00000000-0000-4000-8000-000000000303";
const ENTRY_D_ID: &str = "00000000-0000-4000-8000-000000000304";
const ENTRY_E_ID: &str = "00000000-0000-4000-8000-000000000305";

#[tokio::test]
async fn sqlite_bulk_read_contract() {
    let data = TempDir::new().expect("temporary directory should create");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("reader-bulk-read.db").display()
    );
    bulk_read_contract(SecretString::from(url), "sqlite").await;
}

#[tokio::test]
async fn postgres_bulk_read_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("PostgreSQL bulk read contract skipped: test URL is not configured");
        return;
    };
    bulk_read_contract(SecretString::from(url), "postgres").await;
}

#[tokio::test]
async fn mysql_bulk_read_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("MySQL bulk read contract skipped: test URL is not configured");
        return;
    };
    bulk_read_contract(SecretString::from(url), "mysql").await;
}

async fn bulk_read_contract(url: SecretString, backend_name: &str) {
    let database = connect_for_contract(url).await;
    let _ = rollback(&database).await;
    migrate(&database)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} migrations should apply"));
    seed_contract(&database).await;
    let repository = FeedRepository::new(database.clone());

    assert!(matches!(
        repository
            .mark_read_for_user(USER_A_ID, MarkReadScope::All, 4)
            .await,
        Err(RepositoryError::InvalidSnapshotGeneration)
    ));
    assert_eq!(
        subscription_frontier(&database, SUBSCRIPTION_A_ID).await,
        (0, 0)
    );

    let category_result = repository
        .mark_read_for_user(
            USER_A_ID,
            MarkReadScope::Category(CATEGORY_ID.to_owned()),
            2,
        )
        .await
        .expect("category snapshot should mark read");
    assert_eq!(category_result.changed_subscriptions, 1);
    assert_eq!(
        subscription_frontier(&database, SUBSCRIPTION_A_ID).await,
        (2, 1)
    );
    assert_eq!(
        subscription_frontier(&database, SUBSCRIPTION_B_ID).await,
        (0, 0)
    );
    assert!(entry_state_for(&database, ENTRY_A_ID).await.is_none());
    let starred = entry_state_for(&database, ENTRY_B_ID)
        .await
        .expect("starred override should remain");
    assert_eq!(starred.read_override, None);
    assert!(starred.is_starred);
    let later = entry_state_for(&database, ENTRY_C_ID)
        .await
        .expect("later explicit unread should remain");
    assert_eq!(later.read_override, Some(false));

    let unread = repository
        .list_for_user(
            USER_A_ID,
            ListEntriesQuery {
                state: EntryListState::Unread,
                feed_id: Some(FEED_ID.to_owned()),
                ..ListEntriesQuery::default()
            },
        )
        .await
        .expect("unread Feed should query");
    assert_eq!(
        unread
            .items
            .iter()
            .map(|item| item.entry_id.as_str())
            .collect::<Vec<_>>(),
        [ENTRY_C_ID]
    );

    assert_eq!(
        repository
            .mark_read_for_user(
                USER_A_ID,
                MarkReadScope::Category(CATEGORY_ID.to_owned()),
                2,
            )
            .await
            .expect("repeat category snapshot should succeed")
            .changed_subscriptions,
        0
    );

    assert_eq!(
        repository
            .mark_read_for_user(USER_A_ID, MarkReadScope::Feed(FEED_B_ID.to_owned()), 1,)
            .await
            .expect("Feed snapshot should mark read")
            .changed_subscriptions,
        1
    );
    assert_eq!(
        subscription_frontier(&database, SUBSCRIPTION_B_ID).await,
        (1, 1)
    );

    assert_eq!(
        repository
            .mark_read_for_user(USER_A_ID, MarkReadScope::All, 3)
            .await
            .expect("all snapshot should mark read")
            .changed_subscriptions,
        2
    );
    assert_eq!(
        subscription_frontier(&database, SUBSCRIPTION_A_ID).await,
        (3, 2)
    );
    assert_eq!(
        subscription_frontier(&database, SUBSCRIPTION_B_ID).await,
        (2, 2)
    );
    assert!(entry_state_for(&database, ENTRY_C_ID).await.is_none());

    assert_eq!(
        repository
            .mark_read_for_user(USER_A_ID, MarkReadScope::All, 3)
            .await
            .expect("repeat all snapshot should succeed")
            .changed_subscriptions,
        0
    );
    assert_eq!(
        repository
            .mark_read_for_user(USER_B_ID, MarkReadScope::Feed(FEED_ID.to_owned()), 3,)
            .await
            .expect("cross-user Feed scope should be hidden")
            .changed_subscriptions,
        0
    );
    assert_eq!(
        repository
            .mark_read_for_user(
                USER_A_ID,
                MarkReadScope::Category("00000000-0000-4000-8000-000000000599".to_owned(),),
                3,
            )
            .await
            .expect("empty category scope should succeed")
            .changed_subscriptions,
        0
    );

    for (scope, generation, expected) in [
        (MarkReadScope::Feed("not-a-uuid".to_owned()), 1, "feed"),
        (
            MarkReadScope::Category("not-a-uuid".to_owned()),
            1,
            "category",
        ),
        (MarkReadScope::All, -1, "snapshot"),
    ] {
        let error = repository
            .mark_read_for_user(USER_A_ID, scope, generation)
            .await
            .expect_err("invalid bulk read input should fail");
        match expected {
            "feed" => assert!(matches!(error, RepositoryError::InvalidFeedId)),
            "category" => assert!(matches!(error, RepositoryError::InvalidCategoryId)),
            "snapshot" => assert!(matches!(error, RepositoryError::InvalidSnapshotGeneration)),
            _ => unreachable!(),
        }
    }

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_individual_and_bulk_read_converge_to_a_serial_order() {
    let data = TempDir::new().expect("temporary directory should create");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("reader-bulk-race.db").display()
    );
    let database = connect_for_contract(SecretString::from(url)).await;
    migrate(&database).await.expect("migrations should apply");
    let now = OffsetDateTime::now_utc();
    insert_user(&database, USER_A_ID, "bulk-race").await;
    insert_feed(&database, now).await;
    let mut subscription = subscription_model(SUBSCRIPTION_A_ID, USER_A_ID, now);
    subscription.start_sequence = Set(0);
    subscription
        .insert(&database)
        .await
        .expect("subscription should insert");
    seed_entry(&database, ENTRY_A_ID, FEED_ID, 1, 1, now).await;
    set_generation(&database, 1).await;

    let first = FeedRepository::new(database.clone());
    let second = FeedRepository::new(database.clone());
    let (single, bulk) = tokio::join!(
        first.update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: Some(false),
                is_starred: None,
            },
        ),
        second.mark_read_for_user(USER_A_ID, MarkReadScope::All, 1),
    );
    assert!(
        single
            .expect("individual update should serialize")
            .is_some()
    );
    assert_eq!(
        bulk.expect("bulk update should serialize")
            .changed_subscriptions,
        1
    );
    assert_eq!(
        subscription_frontier(&database, SUBSCRIPTION_A_ID).await.0,
        1
    );

    let state = entry_state_for(&database, ENTRY_A_ID).await;
    assert!(
        state
            .as_ref()
            .is_none_or(|state| { state.read_override == Some(false) && !state.is_starred })
    );
    let unread = second
        .list_for_user(
            USER_A_ID,
            ListEntriesQuery {
                state: EntryListState::Unread,
                feed_id: Some(FEED_ID.to_owned()),
                ..ListEntriesQuery::default()
            },
        )
        .await
        .expect("serialized unread state should query");
    assert_eq!(unread.items.len(), usize::from(state.is_some()));

    database.close().await.expect("database should close");
}

async fn seed_contract(database: &DatabaseConnection) {
    let now = OffsetDateTime::now_utc();
    insert_user(database, USER_A_ID, "bulk-reader").await;
    insert_user(database, USER_B_ID, "bulk-other").await;
    insert_feed(database, now).await;

    let stored_feed = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("source Feed should query")
        .expect("source Feed should exist");
    let mut first_feed = stored_feed.clone().into_active_model();
    first_feed.entry_sequence_head = Set(3);
    first_feed
        .update(database)
        .await
        .expect("first Feed head should update");
    let mut second_feed = stored_feed.into_active_model();
    second_feed.id = Set(FEED_B_ID.to_owned());
    second_feed.source_url = Set("https://second.example.test/feed.xml".to_owned());
    second_feed.normalized_url = Set("https://second.example.test/feed.xml".to_owned());
    second_feed.normalized_url_hash = Set(HASH_C.to_owned());
    second_feed.fetch_url = Set("https://second.example.test/feed.xml".to_owned());
    second_feed.validator_url = Set(Some("https://second.example.test/feed.xml".to_owned()));
    second_feed.entry_sequence_head = Set(2);
    second_feed
        .insert(database)
        .await
        .expect("second Feed should insert");

    category::ActiveModel {
        id: Set(CATEGORY_ID.to_owned()),
        user_id: Set(USER_A_ID.to_owned()),
        title: Set("Technology".to_owned()),
        normalized_title: Set("technology".to_owned()),
        position: Set(1024),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(database)
    .await
    .expect("category should insert");

    let mut first_subscription = subscription_model(SUBSCRIPTION_A_ID, USER_A_ID, now);
    first_subscription.start_sequence = Set(0);
    first_subscription.category_id = Set(Some(CATEGORY_ID.to_owned()));
    first_subscription
        .insert(database)
        .await
        .expect("first Subscription should insert");
    let mut second_subscription = subscription_model(SUBSCRIPTION_B_ID, USER_A_ID, now);
    second_subscription.feed_id = Set(FEED_B_ID.to_owned());
    second_subscription.start_sequence = Set(0);
    second_subscription
        .insert(database)
        .await
        .expect("second Subscription should insert");

    for (entry_id, feed_id, sequence, generation) in [
        (ENTRY_A_ID, FEED_ID, 1, 1),
        (ENTRY_B_ID, FEED_ID, 2, 2),
        (ENTRY_C_ID, FEED_ID, 3, 3),
        (ENTRY_D_ID, FEED_B_ID, 1, 1),
        (ENTRY_E_ID, FEED_B_ID, 2, 3),
    ] {
        seed_entry(database, entry_id, feed_id, sequence, generation, now).await;
    }
    seed_state(database, ENTRY_A_ID, FEED_ID, 1, false, now).await;
    seed_state(database, ENTRY_B_ID, FEED_ID, 2, true, now).await;
    seed_state(database, ENTRY_C_ID, FEED_ID, 3, false, now).await;
    seed_state(database, ENTRY_D_ID, FEED_B_ID, 1, false, now).await;
    set_generation(database, 3).await;
}

async fn seed_entry(
    database: &DatabaseConnection,
    entry_id: &str,
    feed_id: &str,
    sequence: i64,
    generation: i64,
    now: OffsetDateTime,
) {
    let identity = format!("bulk-entry-{feed_id}-{sequence}");
    let mut model = entry_model(
        entry_id,
        sequence,
        &identity,
        blake3::hash(identity.as_bytes()).to_hex().as_ref(),
        Some(1_800_000_000_000_000 + sequence),
        now,
    );
    model.feed_id = Set(feed_id.to_owned());
    model.ingest_generation = Set(generation);
    model.title = Set(Some(format!("Bulk {sequence}")));
    model.search_text = Set(format!("bulk {sequence}"));
    model
        .insert(database)
        .await
        .expect("bulk Entry should insert");
}

async fn seed_state(
    database: &DatabaseConnection,
    entry_id: &str,
    feed_id: &str,
    sequence: i64,
    starred: bool,
    now: OffsetDateTime,
) {
    entry_state::ActiveModel {
        user_id: Set(USER_A_ID.to_owned()),
        entry_id: Set(entry_id.to_owned()),
        feed_id: Set(feed_id.to_owned()),
        feed_sequence: Set(sequence),
        read_override: Set(Some(false)),
        is_starred: Set(starred),
        starred_at: Set(starred.then_some(now)),
        revision: Set(1),
        updated_at: Set(now),
    }
    .insert(database)
    .await
    .expect("bulk Entry state should insert");
}

async fn set_generation(database: &DatabaseConnection, generation: i64) {
    let stored = rss_counter::Entity::find_by_id("INGEST_GENERATION")
        .one(database)
        .await
        .expect("generation should query")
        .expect("generation should exist");
    let mut active = stored.into_active_model();
    active.value = Set(generation);
    active
        .update(database)
        .await
        .expect("generation should update");
}

async fn subscription_frontier(database: &DatabaseConnection, id: &str) -> (i64, i64) {
    let subscription = subscription::Entity::find_by_id(id)
        .one(database)
        .await
        .expect("Subscription should query")
        .expect("Subscription should exist");
    (
        subscription.read_through_sequence,
        subscription.state_revision,
    )
}

async fn entry_state_for(
    database: &DatabaseConnection,
    entry_id: &str,
) -> Option<entry_state::Model> {
    entry_state::Entity::find()
        .filter(entry_state::Column::UserId.eq(USER_A_ID))
        .filter(entry_state::Column::EntryId.eq(entry_id))
        .one(database)
        .await
        .expect("Entry state should query")
}
