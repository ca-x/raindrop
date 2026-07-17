mod support;

use raindrop::db::{
    entities::{entry, entry_state, feed, feed_refresh_run, rss_counter, subscription, user},
    migrate, rollback,
};
use raindrop::feeds::EntryContentDetail;
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, Statement,
    sea_query::{Alias, Expr, Query},
};
use sea_orm_migration::SchemaManager;
use secrecy::SecretString;
use support::database::{
    ENTRY_A_ID, ENTRY_B_ID, FEED_ID, HASH_B, HASH_C, SUBSCRIPTION_A_ID, SUBSCRIPTION_B_ID,
    USER_A_ID, USER_B_ID, connect_for_contract, entry_model, insert_entry, insert_entry_state,
    insert_feed, insert_subscription, insert_user, subscription_model,
};
use tempfile::tempdir;
use time::{OffsetDateTime, macros::datetime};

const ROUNDTRIP_AT: OffsetDateTime = datetime!(2040-02-03 04:05:06.123456 UTC);
const BEFORE_UNIX_EPOCH_US: i64 = -1;
const AFTER_2038_US: i64 = 2_147_483_648_000_001;

#[tokio::test]
async fn sqlite_rss_schema_contract() {
    let data = tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("rss-contract.db").display()
    );

    rss_schema_contract(SecretString::from(url)).await;
}

#[tokio::test]
async fn sqlite_memory_connection_does_not_force_wal() {
    let database = connect_for_contract(SecretString::from("sqlite::memory:".to_owned())).await;
    let journal_mode: String = sea_orm::sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(database.get_sqlite_connection_pool())
        .await
        .expect("in-memory SQLite journal_mode should query");
    assert_ne!(journal_mode, "wal");
    database
        .close()
        .await
        .expect("in-memory SQLite database should close");
}

#[tokio::test]
async fn postgres_rss_schema_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("postgres RSS schema contract skipped: test database URL is not configured");
        return;
    };

    rss_schema_contract(SecretString::from(url)).await;
}

#[tokio::test]
async fn mysql_rss_schema_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("mysql RSS schema contract skipped: test database URL is not configured");
        return;
    };

    rss_schema_contract(SecretString::from(url)).await;
}

async fn rss_schema_contract(database_url: SecretString) {
    let database = connect_for_contract(database_url).await;
    rollback(&database)
        .await
        .unwrap_or_else(|_| panic!("dedicated RSS contract database should reset"));
    migrate(&database)
        .await
        .unwrap_or_else(|_| panic!("RSS migrations should apply"));
    migrate(&database)
        .await
        .unwrap_or_else(|_| panic!("RSS migrations should be idempotent"));

    assert_generation(&database, 0).await;
    assert_expected_indexes(&database).await;
    assert_operational_timestamp_schema(&database).await;
    assert_entry_storage_physical_schema(&database).await;
    assert_feed_metadata_schema(&database).await;
    assert_multiple_pool_connections_use_utc(&database).await;

    insert_user(&database, USER_A_ID, "reader-a").await;
    insert_user(&database, USER_B_ID, "reader-b").await;
    insert_feed(&database, ROUNDTRIP_AT).await;
    assert_feed_metadata_upgrade_reentry(&database).await;
    insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, ROUNDTRIP_AT).await;
    refresh_run_model(
        "00000000-0000-4000-8000-000000000401",
        FEED_ID,
        Some(USER_A_ID),
        "manual:stable-key",
    )
    .insert(&database)
    .await
    .expect("refresh run should insert");
    assert!(
        refresh_run_model(
            "00000000-0000-4000-8000-000000000402",
            FEED_ID,
            Some(USER_A_ID),
            "manual:stable-key",
        )
        .insert(&database)
        .await
        .is_err()
    );

    let duplicate_subscription = subscription_model(
        "00000000-0000-4000-8000-000000000203",
        USER_A_ID,
        ROUNDTRIP_AT,
    )
    .insert(&database)
    .await;
    assert!(duplicate_subscription.is_err());
    assert_eq!(
        feed::Entity::find()
            .count(&database)
            .await
            .expect("feeds should count"),
        1
    );
    insert_entry(
        &database,
        ENTRY_A_ID,
        1,
        "urn:Example:Entry",
        HASH_B,
        Some(BEFORE_UNIX_EPOCH_US),
        ROUNDTRIP_AT,
    )
    .await;
    insert_entry(
        &database,
        ENTRY_B_ID,
        2,
        "urn:example:entry",
        HASH_C,
        Some(AFTER_2038_US),
        ROUNDTRIP_AT,
    )
    .await;

    let duplicate_hash = entry_model(
        "00000000-0000-4000-8000-000000000303",
        3,
        "urn:collision:full-value",
        HASH_B,
        None,
        ROUNDTRIP_AT,
    )
    .insert(&database)
    .await;
    assert!(duplicate_hash.is_err());
    assert_eq!(
        entry::Entity::find()
            .filter(entry::Column::FeedId.eq(FEED_ID))
            .count(&database)
            .await
            .expect("entries should count"),
        2
    );

    let before_epoch = entry::Entity::find_by_id(ENTRY_A_ID)
        .one(&database)
        .await
        .expect("pre-epoch entry should query")
        .expect("pre-epoch entry should exist");
    let after_2038 = entry::Entity::find_by_id(ENTRY_B_ID)
        .one(&database)
        .await
        .expect("post-2038 entry should query")
        .expect("post-2038 entry should exist");
    assert_eq!(before_epoch.published_at_us, Some(BEFORE_UNIX_EPOCH_US));
    assert_eq!(after_2038.published_at_us, Some(AFTER_2038_US));
    assert!(EntryContentDetail::decode(&before_epoch.sanitized_content).is_ok());
    assert!(EntryContentDetail::decode(&after_2038.sanitized_content).is_ok());

    assert_entry_storage_reentry(&database).await;

    assert!(
        insert_entry_state(&database, USER_B_ID, 1, ROUNDTRIP_AT)
            .await
            .is_err()
    );
    insert_subscription(&database, SUBSCRIPTION_B_ID, USER_B_ID, ROUNDTRIP_AT).await;
    assert!(
        insert_entry_state(&database, USER_B_ID, 2, ROUNDTRIP_AT)
            .await
            .is_err()
    );
    insert_entry_state(&database, USER_A_ID, 1, ROUNDTRIP_AT)
        .await
        .expect("user A state should insert");
    insert_entry_state(&database, USER_B_ID, 1, ROUNDTRIP_AT)
        .await
        .expect("user B state should insert");

    assert_operational_timestamp_roundtrip(&database).await;

    user::Entity::delete_by_id(USER_A_ID)
        .exec(&database)
        .await
        .expect("user A should delete");
    let refresh_after_requester_delete =
        feed_refresh_run::Entity::find_by_id("00000000-0000-4000-8000-000000000401")
            .one(&database)
            .await
            .expect("refresh run should query after requester deletion")
            .expect("refresh run should survive requester deletion");
    assert_eq!(refresh_after_requester_delete.requested_by_user_id, None);
    assert_eq!(
        subscription::Entity::find()
            .filter(subscription::Column::UserId.eq(USER_A_ID))
            .count(&database)
            .await
            .expect("user A subscriptions should count"),
        0
    );
    assert_eq!(
        entry_state::Entity::find()
            .filter(entry_state::Column::UserId.eq(USER_A_ID))
            .count(&database)
            .await
            .expect("user A states should count"),
        0
    );
    assert_eq!(
        subscription::Entity::find()
            .filter(subscription::Column::UserId.eq(USER_B_ID))
            .count(&database)
            .await
            .expect("user B subscriptions should count"),
        1
    );
    assert_eq!(
        entry_state::Entity::find()
            .filter(entry_state::Column::UserId.eq(USER_B_ID))
            .count(&database)
            .await
            .expect("user B states should count"),
        1
    );
    assert_eq!(
        feed::Entity::find()
            .count(&database)
            .await
            .expect("feeds should count"),
        1
    );
    assert_eq!(
        entry::Entity::find()
            .count(&database)
            .await
            .expect("entries should count"),
        2
    );

    if database.get_database_backend() == DatabaseBackend::MySql {
        assert_mysql_partial_index_reentry(&database).await;
    }
    assert_refresh_run_constraints(&database).await;
    assert_counter_seed_reentry(&database).await;

    assert_entry_storage_down_is_fail_closed(&database).await;
    rollback(&database)
        .await
        .unwrap_or_else(|_| panic!("RSS contract database should roll back"));
    migrate(&database)
        .await
        .unwrap_or_else(|_| panic!("RSS migrations should reapply after rollback"));
    assert_feed_metadata_schema(&database).await;
    rollback(&database)
        .await
        .unwrap_or_else(|_| panic!("reapplied RSS contract database should roll back"));
    database
        .close()
        .await
        .unwrap_or_else(|_| panic!("RSS contract database should close"));
}

async fn assert_entry_storage_reentry(database: &DatabaseConnection) {
    let mut batch_ids = Vec::new();
    for index in 0..33_i64 {
        let id = format!("10000000-0000-4000-8000-{index:012}");
        let identity = format!("legacy-batch-{index}");
        let identity_hash = format!("{:064x}", index + 100);
        let mut model = entry_model(
            &id,
            index + 100,
            &identity,
            &identity_hash,
            None,
            ROUNDTRIP_AT,
        );
        model.sanitized_content = Set("<p>Legacy batch content</p>".to_owned());
        model
            .insert(database)
            .await
            .expect("legacy batch fixture should insert");
        batch_ids.push(id);
    }
    database
        .execute(Statement::from_sql_and_values(
            database.get_database_backend(),
            match database.get_database_backend() {
                DatabaseBackend::Postgres => {
                    "UPDATE entries SET sanitized_content = $1 WHERE id = $2"
                }
                DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                    "UPDATE entries SET sanitized_content = ? WHERE id = ?"
                }
            },
            ["rdsc:notes".into(), ENTRY_A_ID.into()],
        ))
        .await
        .expect("legacy bare HTML fixture should update");

    if database.get_database_backend() == DatabaseBackend::MySql {
        database
            .execute(Statement::from_string(
                DatabaseBackend::MySql,
                "ALTER TABLE entries MODIFY COLUMN identity TEXT NOT NULL".to_owned(),
            ))
            .await
            .expect("partial MySQL widening fixture should narrow one column");
    }

    delete_migration_marker(database, "entry_storage").await;
    migrate(database)
        .await
        .expect("entry storage migration should recover partial work");

    let entry = entry::Entity::find_by_id(ENTRY_A_ID)
        .one(database)
        .await
        .expect("backfilled entry should query")
        .expect("backfilled entry should exist");
    let detail = EntryContentDetail::decode(&entry.sanitized_content)
        .expect("legacy HTML should become a valid envelope");
    assert_eq!(detail.html(), "rdsc:notes");
    assert!(detail.inert_images().is_empty());
    for id in &batch_ids {
        let entry = entry::Entity::find_by_id(id)
            .one(database)
            .await
            .expect("batch-backfilled entry should query")
            .expect("batch-backfilled entry should exist");
        let detail = EntryContentDetail::decode(&entry.sanitized_content)
            .expect("each keyset batch row should become a valid envelope");
        assert_eq!(detail.html(), "<p>Legacy batch content</p>");
    }
    assert_entry_storage_physical_schema(database).await;
    for id in batch_ids {
        entry::Entity::delete_by_id(id)
            .exec(database)
            .await
            .expect("legacy batch fixture should delete");
    }
}

async fn assert_entry_storage_down_is_fail_closed(database: &DatabaseConnection) {
    if database.get_database_backend() == DatabaseBackend::MySql {
        database
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::MySql,
                "UPDATE entries SET identity = ? WHERE id = ?",
                ["x".repeat(65_536).into(), ENTRY_A_ID.into()],
            ))
            .await
            .expect("oversized MySQL rollback fixture should update");
        assert!(rollback(database).await.is_err());
        database
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::MySql,
                "UPDATE entries SET identity = ? WHERE id = ?",
                ["urn:Example:Entry".into(), ENTRY_A_ID.into()],
            ))
            .await
            .expect("bounded MySQL rollback fixture should restore");
    }
    let envelope = "rdsc:v1:{\"html\":\"<img alt=\\\"A\\\">\",\"inertImages\":[{\"imageIndex\":0,\"sourceUrl\":\"https://img.example.test/a.jpg\",\"alt\":\"A\",\"width\":null,\"height\":null}]}";
    database
        .execute(Statement::from_sql_and_values(
            database.get_database_backend(),
            match database.get_database_backend() {
                DatabaseBackend::Postgres => {
                    "UPDATE entries SET sanitized_content = $1 WHERE id = $2"
                }
                DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                    "UPDATE entries SET sanitized_content = ? WHERE id = ?"
                }
            },
            [envelope.into(), ENTRY_A_ID.into()],
        ))
        .await
        .expect("inert image rollback fixture should update");
    assert!(rollback(database).await.is_err());

    database
        .execute(Statement::from_sql_and_values(
            database.get_database_backend(),
            match database.get_database_backend() {
                DatabaseBackend::Postgres => {
                    "UPDATE entries SET sanitized_content = $1 WHERE id = $2"
                }
                DatabaseBackend::Sqlite | DatabaseBackend::MySql => {
                    "UPDATE entries SET sanitized_content = ? WHERE id = ?"
                }
            },
            [
                "rdsc:v1:{\"html\":\"<img alt=\\\"A\\\">\",\"inertImages\":[]}".into(),
                ENTRY_A_ID.into(),
            ],
        ))
        .await
        .expect("rollback-safe fixture should restore");
}

async fn assert_entry_storage_physical_schema(database: &DatabaseConnection) {
    if database.get_database_backend() != DatabaseBackend::MySql {
        return;
    }
    let row = database
        .query_one(Statement::from_string(
            DatabaseBackend::MySql,
            "SELECT
                CAST(SUM(column_name = 'sanitized_content' AND data_type = 'longtext') AS SIGNED) AS long_count,
                CAST(SUM(column_name IN ('identity','title','author','summary','enclosure_json') AND data_type = 'mediumtext') AS SIGNED) AS medium_count
             FROM information_schema.columns
             WHERE table_schema = DATABASE() AND table_name = 'entries'"
                .to_owned(),
        ))
        .await
        .expect("MySQL entry storage column types should query")
        .expect("MySQL entry storage column type counts should exist");
    let long_count: i64 = row.try_get("", "long_count").expect("LONGTEXT count");
    let medium_count: i64 = row.try_get("", "medium_count").expect("MEDIUMTEXT count");
    assert_eq!(long_count, 1);
    assert_eq!(medium_count, 5);
}

async fn assert_feed_metadata_schema(database: &DatabaseConnection) {
    let manager = SchemaManager::new(database);
    assert!(
        manager
            .has_column("feeds", "title")
            .await
            .expect("feed title column should query")
    );
    assert!(
        manager
            .has_column("feeds", "site_url")
            .await
            .expect("feed site URL column should query")
    );
    if database.get_database_backend() != DatabaseBackend::MySql {
        return;
    }
    let row = database
        .query_one(Statement::from_string(
            DatabaseBackend::MySql,
            "SELECT
                CAST(SUM(column_name = 'title' AND data_type = 'mediumtext' AND is_nullable = 'YES') AS SIGNED) AS title_count,
                CAST(SUM(column_name = 'site_url' AND data_type = 'text' AND is_nullable = 'YES') AS SIGNED) AS site_url_count
             FROM information_schema.columns
             WHERE table_schema = DATABASE() AND table_name = 'feeds'
               AND column_name IN ('title', 'site_url')"
                .to_owned(),
        ))
        .await
        .expect("MySQL feed metadata column types should query")
        .expect("MySQL feed metadata column type counts should exist");
    let title_count: i64 = row
        .try_get("", "title_count")
        .expect("MEDIUMTEXT title count");
    let site_url_count: i64 = row
        .try_get("", "site_url_count")
        .expect("TEXT site URL count");
    assert_eq!(title_count, 1);
    assert_eq!(site_url_count, 1);
}

async fn assert_feed_metadata_upgrade_reentry(database: &DatabaseConnection) {
    delete_migration_marker(database, "feed_metadata").await;
    database
        .execute(Statement::from_string(
            database.get_database_backend(),
            "ALTER TABLE feeds DROP COLUMN title".to_owned(),
        ))
        .await
        .expect("legacy feed schema fixture should drop title");
    database
        .execute(Statement::from_string(
            database.get_database_backend(),
            "ALTER TABLE feeds DROP COLUMN site_url".to_owned(),
        ))
        .await
        .expect("legacy feed schema fixture should drop site URL");

    migrate(database)
        .await
        .expect("existing feed schema should gain display metadata additively");
    assert_feed_metadata_schema(database).await;
    let feed = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("upgraded feed should query")
        .expect("upgraded feed should remain present");
    assert_eq!(feed.title, None);
    assert_eq!(feed.site_url, None);
}

async fn assert_operational_timestamp_roundtrip(database: &DatabaseConnection) {
    let feed = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("feed timestamp should query")
        .expect("feed should exist");
    assert_eq!(feed.last_attempt_at, Some(ROUNDTRIP_AT));
    assert_eq!(feed.last_success_at, Some(ROUNDTRIP_AT));
    assert_eq!(feed.last_changed_at, Some(ROUNDTRIP_AT));
    assert_eq!(feed.created_at, ROUNDTRIP_AT);
    assert_eq!(feed.updated_at, ROUNDTRIP_AT);

    let subscription = subscription::Entity::find_by_id(SUBSCRIPTION_A_ID)
        .one(database)
        .await
        .expect("subscription timestamp should query")
        .expect("subscription should exist");
    assert_eq!(subscription.created_at, ROUNDTRIP_AT);
    assert_eq!(subscription.updated_at, ROUNDTRIP_AT);

    let entry = entry::Entity::find_by_id(ENTRY_A_ID)
        .one(database)
        .await
        .expect("entry timestamp should query")
        .expect("entry should exist");
    assert_eq!(entry.inserted_at, ROUNDTRIP_AT);
    assert_eq!(entry.updated_at, ROUNDTRIP_AT);

    let state = entry_state::Entity::find_by_id((USER_A_ID.to_owned(), ENTRY_A_ID.to_owned()))
        .one(database)
        .await
        .expect("entry state timestamp should query")
        .expect("entry state should exist");
    assert_eq!(state.starred_at, Some(ROUNDTRIP_AT));
    assert_eq!(state.updated_at, ROUNDTRIP_AT);

    let refresh = feed_refresh_run::Entity::find_by_id("00000000-0000-4000-8000-000000000401")
        .one(database)
        .await
        .expect("refresh timestamp should query")
        .expect("refresh should exist");
    assert_eq!(refresh.queued_at, ROUNDTRIP_AT);
    assert_eq!(refresh.started_at, Some(ROUNDTRIP_AT));
    assert_eq!(refresh.fetched_at, Some(ROUNDTRIP_AT));
    assert_eq!(refresh.persisted_at, Some(ROUNDTRIP_AT));
    assert_eq!(refresh.completed_at, Some(ROUNDTRIP_AT));
    assert_eq!(refresh.retry_at, Some(ROUNDTRIP_AT));
}

fn refresh_run_model(
    id: &str,
    feed_id: &str,
    requested_by_user_id: Option<&str>,
    idempotency_key: &str,
) -> feed_refresh_run::ActiveModel {
    feed_refresh_run::ActiveModel {
        id: Set(id.to_owned()),
        feed_id: Set(feed_id.to_owned()),
        requested_by_user_id: Set(requested_by_user_id.map(str::to_owned)),
        trigger_kind: Set("MANUAL".to_owned()),
        status: Set("RUNNING".to_owned()),
        idempotency_key: Set(idempotency_key.to_owned()),
        lease_token: Set(Some(2)),
        commit_generation: Set(None),
        queued_at: Set(ROUNDTRIP_AT),
        started_at: Set(Some(ROUNDTRIP_AT)),
        fetched_at: Set(Some(ROUNDTRIP_AT)),
        persisted_at: Set(Some(ROUNDTRIP_AT)),
        completed_at: Set(Some(ROUNDTRIP_AT)),
        http_status: Set(Some(200)),
        new_count: Set(3),
        updated_count: Set(2),
        dropped_count: Set(1),
        error_code: Set(Some("ROUNDTRIP".to_owned())),
        retry_at: Set(Some(ROUNDTRIP_AT)),
    }
}

async fn assert_refresh_run_constraints(database: &DatabaseConnection) {
    const SECOND_FEED_ID: &str = "00000000-0000-4000-8000-000000000102";
    let original = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("source feed should query")
        .expect("source feed should exist");
    let mut second: feed::ActiveModel = original.into();
    second.id = Set(SECOND_FEED_ID.to_owned());
    second.normalized_url_hash = Set(HASH_C.to_owned());
    second
        .insert(database)
        .await
        .expect("second feed should insert");

    refresh_run_model(
        "00000000-0000-4000-8000-000000000402",
        SECOND_FEED_ID,
        None,
        "manual:stable-key",
    )
    .insert(database)
    .await
    .expect("same idempotency key should be reusable for another feed");

    let first = feed_refresh_run::Entity::find_by_id("00000000-0000-4000-8000-000000000401")
        .one(database)
        .await
        .expect("first refresh should query")
        .expect("first refresh should exist");
    let mut first: feed_refresh_run::ActiveModel = first.into();
    first.commit_generation = Set(Some(42));
    first
        .update(database)
        .await
        .expect("first generation should update");

    let second_run = feed_refresh_run::Entity::find_by_id("00000000-0000-4000-8000-000000000402")
        .one(database)
        .await
        .expect("second refresh should query")
        .expect("second refresh should exist");
    let mut second_run: feed_refresh_run::ActiveModel = second_run.into();
    second_run.commit_generation = Set(Some(42));
    assert!(second_run.update(database).await.is_err());

    feed::Entity::delete_by_id(SECOND_FEED_ID)
        .exec(database)
        .await
        .expect("second feed should delete");
    assert!(
        feed_refresh_run::Entity::find_by_id("00000000-0000-4000-8000-000000000402")
            .one(database)
            .await
            .expect("cascaded refresh should query")
            .is_none()
    );
}

async fn assert_multiple_pool_connections_use_utc(database: &DatabaseConnection) {
    match database.get_database_backend() {
        DatabaseBackend::Postgres => {
            let pool = database.get_postgres_connection_pool();
            let mut first = pool
                .acquire()
                .await
                .unwrap_or_else(|_| panic!("first PostgreSQL pool connection should acquire"));
            let mut second = pool
                .acquire()
                .await
                .unwrap_or_else(|_| panic!("second PostgreSQL pool connection should acquire"));
            let first_zone: String = sea_orm::sqlx::query_scalar("SHOW TIME ZONE")
                .fetch_one(&mut *first)
                .await
                .unwrap_or_else(|_| panic!("first PostgreSQL timezone should query"));
            let second_zone: String = sea_orm::sqlx::query_scalar("SHOW TIME ZONE")
                .fetch_one(&mut *second)
                .await
                .unwrap_or_else(|_| panic!("second PostgreSQL timezone should query"));
            assert_eq!(first_zone, "UTC");
            assert_eq!(second_zone, "UTC");
        }
        DatabaseBackend::MySql => {
            let pool = database.get_mysql_connection_pool();
            let mut first = pool
                .acquire()
                .await
                .unwrap_or_else(|_| panic!("first MySQL pool connection should acquire"));
            let mut second = pool
                .acquire()
                .await
                .unwrap_or_else(|_| panic!("second MySQL pool connection should acquire"));
            let first_zone: String = sea_orm::sqlx::query_scalar("SELECT @@session.time_zone")
                .fetch_one(&mut *first)
                .await
                .unwrap_or_else(|_| panic!("first MySQL timezone should query"));
            let second_zone: String = sea_orm::sqlx::query_scalar("SELECT @@session.time_zone")
                .fetch_one(&mut *second)
                .await
                .unwrap_or_else(|_| panic!("second MySQL timezone should query"));
            assert_eq!(first_zone, "+00:00");
            assert_eq!(second_zone, "+00:00");
        }
        DatabaseBackend::Sqlite => {
            let pool = database.get_sqlite_connection_pool();
            let mut first = pool
                .acquire()
                .await
                .unwrap_or_else(|_| panic!("first SQLite pool connection should acquire"));
            let first_foreign_keys: i64 = sea_orm::sqlx::query_scalar("PRAGMA foreign_keys")
                .fetch_one(&mut *first)
                .await
                .expect("first SQLite foreign_keys setting should query");
            let first_busy_timeout: i64 = sea_orm::sqlx::query_scalar("PRAGMA busy_timeout")
                .fetch_one(&mut *first)
                .await
                .expect("first SQLite busy_timeout setting should query");
            let first_synchronous: i64 = sea_orm::sqlx::query_scalar("PRAGMA synchronous")
                .fetch_one(&mut *first)
                .await
                .expect("first SQLite synchronous setting should query");
            let first_journal_mode: String = sea_orm::sqlx::query_scalar("PRAGMA journal_mode")
                .fetch_one(&mut *first)
                .await
                .expect("first SQLite journal_mode setting should query");
            assert_eq!(first_foreign_keys, 1);
            assert_eq!(first_busy_timeout, 5_000);
            assert_eq!(first_synchronous, 1);
            assert_eq!(first_journal_mode, "wal");
            first
                .close()
                .await
                .expect("first SQLite connection should close");

            let mut second = pool
                .acquire()
                .await
                .unwrap_or_else(|_| panic!("replacement SQLite pool connection should acquire"));
            let second_foreign_keys: i64 = sea_orm::sqlx::query_scalar("PRAGMA foreign_keys")
                .fetch_one(&mut *second)
                .await
                .expect("replacement SQLite foreign_keys setting should query");
            let second_busy_timeout: i64 = sea_orm::sqlx::query_scalar("PRAGMA busy_timeout")
                .fetch_one(&mut *second)
                .await
                .expect("replacement SQLite busy_timeout setting should query");
            let second_synchronous: i64 = sea_orm::sqlx::query_scalar("PRAGMA synchronous")
                .fetch_one(&mut *second)
                .await
                .expect("replacement SQLite synchronous setting should query");
            let second_journal_mode: String = sea_orm::sqlx::query_scalar("PRAGMA journal_mode")
                .fetch_one(&mut *second)
                .await
                .expect("replacement SQLite journal_mode setting should query");
            assert_eq!(second_foreign_keys, 1);
            assert_eq!(second_busy_timeout, 5_000);
            assert_eq!(second_synchronous, 1);
            assert_eq!(second_journal_mode, "wal");
        }
    }
}

async fn assert_operational_timestamp_schema(database: &DatabaseConnection) {
    let query = match database.get_database_backend() {
        DatabaseBackend::MySql => Some(
            "SELECT CAST(COUNT(*) AS SIGNED) AS matching_count
             FROM information_schema.columns
             WHERE table_schema = DATABASE()
               AND data_type = 'datetime'
               AND datetime_precision = 6
               AND (
                 (table_name = 'feeds' AND column_name IN ('last_attempt_at','last_success_at','last_changed_at','next_fetch_at','retry_after_at','orphaned_at','lease_until','created_at','updated_at'))
                 OR (table_name = 'subscriptions' AND column_name IN ('created_at','updated_at'))
                 OR (table_name = 'entries' AND column_name IN ('inserted_at','updated_at'))
                 OR (table_name = 'entry_states' AND column_name IN ('starred_at','updated_at'))
                 OR (table_name = 'feed_refresh_runs' AND column_name IN ('queued_at','started_at','fetched_at','persisted_at','completed_at','retry_at'))
               )",
        ),
        DatabaseBackend::Postgres => Some(
            "SELECT COUNT(*)::BIGINT AS matching_count
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND data_type = 'timestamp with time zone'
               AND (
                 (table_name = 'feeds' AND column_name IN ('last_attempt_at','last_success_at','last_changed_at','next_fetch_at','retry_after_at','orphaned_at','lease_until','created_at','updated_at'))
                 OR (table_name = 'subscriptions' AND column_name IN ('created_at','updated_at'))
                 OR (table_name = 'entries' AND column_name IN ('inserted_at','updated_at'))
                 OR (table_name = 'entry_states' AND column_name IN ('starred_at','updated_at'))
                 OR (table_name = 'feed_refresh_runs' AND column_name IN ('queued_at','started_at','fetched_at','persisted_at','completed_at','retry_at'))
               )",
        ),
        DatabaseBackend::Sqlite => None,
    };

    if let Some(query) = query {
        let row = database
            .query_one(Statement::from_string(
                database.get_database_backend(),
                query.to_owned(),
            ))
            .await
            .expect("operational timestamp schema should query")
            .expect("operational timestamp schema count should exist");
        let matching_count: i64 = row
            .try_get("", "matching_count")
            .expect("operational timestamp count should decode");
        assert_eq!(matching_count, 21);
    }
}

async fn assert_expected_indexes(database: &DatabaseConnection) {
    let manager = SchemaManager::new(database);
    for (table, index) in [
        ("feeds", "uq_feeds_url_hash"),
        ("feeds", "idx_feeds_due"),
        ("subscriptions", "uq_subscriptions_user_feed"),
        ("subscriptions", "idx_subscriptions_user_pos"),
        ("subscriptions", "idx_subscriptions_feed"),
        ("entries", "uq_entries_feed_identity"),
        ("entries", "uq_entries_feed_seq"),
        ("entries", "uq_entries_state_tuple"),
        ("entries", "idx_entries_feed_list"),
        ("entries", "idx_entries_all_list"),
        ("entries", "idx_entries_snapshot"),
        ("entry_states", "idx_states_feed_read"),
        ("entry_states", "idx_states_starred"),
        ("feed_refresh_runs", "uq_refresh_runs_idem"),
        ("feed_refresh_runs", "uq_refresh_runs_generation"),
        ("feed_refresh_runs", "idx_refresh_runs_feed"),
        ("feed_refresh_runs", "idx_refresh_runs_status"),
    ] {
        assert!(
            manager
                .has_index(table, index)
                .await
                .expect("named RSS index should query"),
            "missing named RSS index {index}"
        );
    }
}

async fn assert_mysql_partial_index_reentry(database: &DatabaseConnection) {
    database
        .execute(Statement::from_string(
            DatabaseBackend::MySql,
            "DROP TABLE entry_states".to_owned(),
        ))
        .await
        .expect("entry states should drop for partial migration fixture");
    database
        .execute(Statement::from_string(
            DatabaseBackend::MySql,
            "CREATE TABLE entry_states (
                user_id VARCHAR(36) NOT NULL,
                entry_id VARCHAR(36) NOT NULL,
                feed_id VARCHAR(36) NOT NULL,
                feed_sequence BIGINT NOT NULL,
                read_override BOOLEAN NULL,
                is_starred BOOLEAN NOT NULL DEFAULT FALSE,
                starred_at DATETIME(6) NULL,
                revision BIGINT NOT NULL DEFAULT 0,
                updated_at DATETIME(6) NOT NULL,
                PRIMARY KEY (user_id, entry_id),
                CONSTRAINT fk_entry_states_subscription FOREIGN KEY (user_id, feed_id)
                    REFERENCES subscriptions (user_id, feed_id) ON DELETE CASCADE,
                CONSTRAINT fk_entry_states_entry FOREIGN KEY (entry_id, feed_id, feed_sequence)
                    REFERENCES entries (id, feed_id, feed_sequence) ON DELETE CASCADE,
                INDEX idx_states_feed_read (user_id, feed_id, read_override, feed_sequence)
            ) ENGINE=InnoDB"
                .to_owned(),
        ))
        .await
        .expect("complete partial-index entry states table should precreate");
    delete_migration_marker(database, "entry_states").await;

    migrate(database)
        .await
        .expect("partial MySQL migration should recover missing indexes");
    assert_expected_indexes(database).await;
    assert_generation(database, 0).await;
}

async fn assert_counter_seed_reentry(database: &DatabaseConnection) {
    set_generation(database, 7).await;
    delete_migration_marker(database, "counters").await;
    migrate(database)
        .await
        .expect("existing non-negative generation should be valid");
    assert_generation(database, 7).await;

    set_generation(database, -1).await;
    delete_migration_marker(database, "counters").await;
    assert!(migrate(database).await.is_err());
    set_generation(database, 7).await;
    migrate(database)
        .await
        .expect("valid generation should recover after rejected seed state");

    if database.get_database_backend() == DatabaseBackend::Sqlite {
        database
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "UPDATE rss_counters SET value = 'not-an-integer' WHERE key = 'INGEST_GENERATION'"
                    .to_owned(),
            ))
            .await
            .expect("SQLite should permit the invalid-type fixture");
        delete_migration_marker(database, "counters").await;
        assert!(migrate(database).await.is_err());
        set_generation(database, 7).await;
        migrate(database)
            .await
            .expect("typed generation should recover after rejected seed state");

        database
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "CREATE TRIGGER fail_rss_counter_seed BEFORE INSERT ON rss_counters
                 BEGIN SELECT RAISE(ABORT, 'unrelated seed failure'); END"
                    .to_owned(),
            ))
            .await
            .expect("unrelated seed failure trigger should create");
        delete_migration_marker(database, "counters").await;
        assert!(migrate(database).await.is_err());
        database
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "DROP TRIGGER fail_rss_counter_seed".to_owned(),
            ))
            .await
            .expect("unrelated seed failure trigger should drop");
        migrate(database)
            .await
            .expect("seed migration should recover after unrelated error is removed");
    }
    assert_generation(database, 7).await;
}

async fn set_generation(database: &DatabaseConnection, value: i64) {
    rss_counter::ActiveModel {
        key: Set("INGEST_GENERATION".to_owned()),
        value: Set(value),
    }
    .update(database)
    .await
    .expect("generation should update");
}

async fn assert_generation(database: &DatabaseConnection, expected: i64) {
    let generation = rss_counter::Entity::find_by_id("INGEST_GENERATION")
        .one(database)
        .await
        .expect("generation counter should query")
        .expect("generation counter should exist");
    assert_eq!(generation.value, expected);
}

async fn delete_migration_marker(database: &DatabaseConnection, version: &str) {
    let statement = Query::delete()
        .from_table(Alias::new("seaql_migrations"))
        .and_where(Expr::col(Alias::new("version")).eq(version))
        .to_owned();
    database
        .execute(database.get_database_backend().build(&statement))
        .await
        .expect("migration marker should delete");
}
