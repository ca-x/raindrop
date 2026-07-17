#[allow(dead_code)]
mod support;

use raindrop::{
    db::{
        entities::{entry_state, subscription},
        migrate, rollback,
    },
    feeds::{FeedRepository, RepositoryError, UpdateEntryState},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectOptions, ConnectionTrait, Database,
    DatabaseBackend, DatabaseConnection, EntityTrait, QueryFilter, QueryResult, Statement,
    TransactionTrait,
};
use secrecy::SecretString;
use support::database::{
    ENTRY_A_ID, ENTRY_B_ID, FEED_ID, HASH_A, SUBSCRIPTION_A_ID, USER_A_ID, USER_B_ID,
    connect_for_contract, insert_entry, insert_feed, insert_subscription, insert_user,
};
use tempfile::TempDir;
use time::{OffsetDateTime, macros::datetime};

const FIXTURE_AT: OffsetDateTime = datetime!(2026-07-17 12:00:00 UTC);

struct StateFixture {
    _data: TempDir,
    url: String,
    database: DatabaseConnection,
    repository: FeedRepository,
}

impl StateFixture {
    async fn sqlite(name: &str) -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join(format!("{name}.db")).display()
        );
        let database = connect_for_contract(SecretString::from(url.clone())).await;
        migrate(&database)
            .await
            .expect("state fixture migrations should apply");
        insert_user(&database, USER_A_ID, "reader-a").await;
        insert_feed(&database, FIXTURE_AT).await;
        insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, FIXTURE_AT).await;
        insert_entry(
            &database,
            ENTRY_A_ID,
            2,
            "entry-a",
            HASH_A,
            Some(1_784_246_400_000_000),
            FIXTURE_AT,
        )
        .await;
        Self {
            _data: data,
            url,
            repository: FeedRepository::new(database.clone()),
            database,
        }
    }

    async fn seed_state(&self, read_override: Option<bool>, is_starred: bool, revision: i64) {
        entry_state::ActiveModel {
            user_id: Set(USER_A_ID.to_owned()),
            entry_id: Set(ENTRY_A_ID.to_owned()),
            feed_id: Set(FEED_ID.to_owned()),
            feed_sequence: Set(2),
            read_override: Set(read_override),
            is_starred: Set(is_starred),
            starred_at: Set(is_starred.then_some(FIXTURE_AT)),
            revision: Set(revision),
            updated_at: Set(FIXTURE_AT),
        }
        .insert(&self.database)
        .await
        .expect("entry state should seed");
    }

    async fn set_read_through_sequence(&self, sequence: i64) {
        let subscription = subscription::Entity::find_by_id(SUBSCRIPTION_A_ID)
            .one(&self.database)
            .await
            .expect("subscription should query")
            .expect("subscription should exist");
        let mut active: subscription::ActiveModel = subscription.into();
        active.read_through_sequence = Set(sequence);
        active
            .update(&self.database)
            .await
            .expect("subscription frontier should update");
    }

    async fn set_start_sequence(&self, sequence: i64) {
        let subscription = subscription::Entity::find_by_id(SUBSCRIPTION_A_ID)
            .one(&self.database)
            .await
            .expect("subscription should query")
            .expect("subscription should exist");
        let mut active: subscription::ActiveModel = subscription.into();
        active.start_sequence = Set(sequence);
        active
            .update(&self.database)
            .await
            .expect("subscription start sequence should update");
    }

    async fn state_row(&self) -> Option<entry_state::Model> {
        entry_state::Entity::find()
            .filter(entry_state::Column::UserId.eq(USER_A_ID))
            .filter(entry_state::Column::EntryId.eq(ENTRY_A_ID))
            .one(&self.database)
            .await
            .expect("entry state should query")
    }
}

#[tokio::test]
async fn sqlite_base_read_request_stores_null_override() {
    let fixture = StateFixture::sqlite("base-read-null-override").await;
    fixture.set_read_through_sequence(2).await;
    fixture.seed_state(Some(false), true, 4).await;

    let state = fixture
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: Some(true),
                is_starred: None,
            },
        )
        .await
        .expect("base read update should remain typed")
        .expect("visible entry should update");

    assert!(state.is_read);
    assert!(state.is_starred);
    let stored = fixture.state_row().await.unwrap();
    assert_eq!(stored.read_override, None);
    assert!(stored.is_starred);
    assert_eq!(stored.revision, 5);
}

#[tokio::test]
async fn sqlite_star_false_to_true_uses_database_timestamp() {
    let fixture = StateFixture::sqlite("star-database-timestamp").await;

    let state = fixture
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: None,
                is_starred: Some(true),
            },
        )
        .await
        .expect("star update should remain typed")
        .expect("visible entry should update");

    assert!(!state.is_read);
    assert!(state.is_starred);
    let stored = fixture.state_row().await.unwrap();
    assert_eq!(stored.read_override, None);
    assert!(stored.is_starred);
    assert!(stored.starred_at.is_some());
    assert_ne!(stored.starred_at, Some(FIXTURE_AT));
    assert_eq!(stored.revision, 1);
}

#[tokio::test]
async fn sqlite_repeated_star_true_preserves_timestamp_and_revision() {
    let fixture = StateFixture::sqlite("repeated-star-idempotent").await;
    fixture.seed_state(None, true, 7).await;

    let state = fixture
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: None,
                is_starred: Some(true),
            },
        )
        .await
        .expect("repeated star should remain typed")
        .expect("visible entry should update");

    assert!(!state.is_read);
    assert!(state.is_starred);
    let stored = fixture.state_row().await.unwrap();
    assert_eq!(stored.starred_at, Some(FIXTURE_AT));
    assert_eq!(stored.revision, 7);
}

#[tokio::test]
async fn sqlite_star_true_to_false_clears_timestamp() {
    let fixture = StateFixture::sqlite("unstar-clears-timestamp").await;
    fixture.seed_state(Some(true), true, 3).await;

    let state = fixture
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: None,
                is_starred: Some(false),
            },
        )
        .await
        .expect("unstar should remain typed")
        .expect("visible entry should update");

    assert!(state.is_read);
    assert!(!state.is_starred);
    let stored = fixture.state_row().await.unwrap();
    assert_eq!(stored.read_override, Some(true));
    assert!(!stored.is_starred);
    assert_eq!(stored.starred_at, None);
    assert_eq!(stored.revision, 4);
}

#[tokio::test]
async fn sqlite_unstarring_starred_only_row_deletes_sparse_state() {
    let fixture = StateFixture::sqlite("unstar-deletes-neutral-row").await;
    fixture.seed_state(None, true, 9).await;

    let state = fixture
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: None,
                is_starred: Some(false),
            },
        )
        .await
        .expect("neutral unstar should remain typed")
        .expect("visible entry should update");

    assert!(!state.is_read);
    assert!(!state.is_starred);
    assert!(fixture.state_row().await.is_none());
}

#[tokio::test]
async fn sqlite_explicit_unread_is_sparse_above_frontier_and_override_below_frontier() {
    let above = StateFixture::sqlite("unread-above-frontier").await;
    let above_state = above
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: Some(false),
                is_starred: None,
            },
        )
        .await
        .expect("above-frontier unread should remain typed")
        .expect("visible entry should update");
    assert!(!above_state.is_read);
    assert!(above.state_row().await.is_none());

    let below = StateFixture::sqlite("unread-below-frontier").await;
    below.set_read_through_sequence(2).await;
    let below_state = below
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: Some(false),
                is_starred: None,
            },
        )
        .await
        .expect("below-frontier unread should remain typed")
        .expect("visible entry should update");
    assert!(!below_state.is_read);
    let stored = below.state_row().await.unwrap();
    assert_eq!(stored.read_override, Some(false));
    assert_eq!(stored.revision, 1);
}

#[tokio::test]
async fn sqlite_empty_patch_returns_invalid_state_patch() {
    let fixture = StateFixture::sqlite("empty-patch").await;

    let result = fixture
        .repository
        .update_state_for_user(USER_A_ID, ENTRY_A_ID, UpdateEntryState::default())
        .await;

    assert!(matches!(result, Err(RepositoryError::InvalidStatePatch)));
    assert!(fixture.state_row().await.is_none());
}

#[tokio::test]
async fn sqlite_malformed_identifiers_return_typed_validation_errors() {
    let fixture = StateFixture::sqlite("malformed-identifiers").await;
    let patch = UpdateEntryState {
        is_read: Some(true),
        is_starred: None,
    };

    assert!(matches!(
        fixture
            .repository
            .update_state_for_user("not-a-user", ENTRY_A_ID, patch.clone())
            .await,
        Err(RepositoryError::InvalidUserId)
    ));
    assert!(matches!(
        fixture
            .repository
            .update_state_for_user(USER_A_ID, "not-an-entry", patch)
            .await,
        Err(RepositoryError::InvalidEntryId)
    ));
}

#[tokio::test]
async fn sqlite_absent_cross_user_and_pre_subscription_entries_are_invisible() {
    let absent = StateFixture::sqlite("absent-entry").await;
    let patch = UpdateEntryState {
        is_read: None,
        is_starred: Some(true),
    };
    assert_eq!(
        absent
            .repository
            .update_state_for_user(USER_A_ID, ENTRY_B_ID, patch.clone())
            .await
            .expect("absent entry should remain typed"),
        None
    );
    assert!(absent.state_row().await.is_none());

    let cross_user = StateFixture::sqlite("cross-user-entry").await;
    insert_user(&cross_user.database, USER_B_ID, "reader-b").await;
    assert_eq!(
        cross_user
            .repository
            .update_state_for_user(USER_B_ID, ENTRY_A_ID, patch.clone())
            .await
            .expect("cross-user entry should remain typed"),
        None
    );
    assert!(cross_user.state_row().await.is_none());

    let pre_subscription = StateFixture::sqlite("pre-subscription-entry").await;
    pre_subscription.set_start_sequence(2).await;
    assert_eq!(
        pre_subscription
            .repository
            .update_state_for_user(USER_A_ID, ENTRY_A_ID, patch)
            .await
            .expect("pre-subscription entry should remain typed"),
        None
    );
    assert!(pre_subscription.state_row().await.is_none());
}

#[tokio::test]
async fn sqlite_first_base_read_and_false_star_patch_does_not_create_row() {
    let fixture = StateFixture::sqlite("first-neutral-patch").await;
    fixture.set_read_through_sequence(2).await;

    let state = fixture
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: Some(true),
                is_starred: Some(false),
            },
        )
        .await
        .expect("neutral patch should remain typed")
        .expect("visible entry should update");

    assert!(state.is_read);
    assert!(!state.is_starred);
    assert!(fixture.state_row().await.is_none());
}

#[tokio::test]
async fn sqlite_clearing_last_read_override_deletes_sparse_state() {
    let fixture = StateFixture::sqlite("clear-last-read-override").await;
    fixture.seed_state(Some(true), false, 6).await;

    let state = fixture
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: Some(false),
                is_starred: None,
            },
        )
        .await
        .expect("read override clear should remain typed")
        .expect("visible entry should update");

    assert!(!state.is_read);
    assert!(!state.is_starred);
    assert!(fixture.state_row().await.is_none());
}

#[tokio::test]
async fn sqlite_clearing_both_fields_deletes_but_clearing_one_preserves_other() {
    let both = StateFixture::sqlite("clear-both-fields").await;
    both.seed_state(Some(true), true, 10).await;
    let both_state = both
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: Some(false),
                is_starred: Some(false),
            },
        )
        .await
        .expect("combined clear should remain typed")
        .expect("visible entry should update");
    assert!(!both_state.is_read);
    assert!(!both_state.is_starred);
    assert!(both.state_row().await.is_none());

    let one = StateFixture::sqlite("clear-read-preserve-star").await;
    one.seed_state(Some(true), true, 10).await;
    let one_state = one
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: Some(false),
                is_starred: None,
            },
        )
        .await
        .expect("partial clear should remain typed")
        .expect("visible entry should update");
    assert!(!one_state.is_read);
    assert!(one_state.is_starred);
    let stored = one.state_row().await.unwrap();
    assert_eq!(stored.read_override, None);
    assert!(stored.is_starred);
    assert_eq!(stored.revision, 11);
}

#[tokio::test]
async fn sqlite_combined_patch_changes_both_fields_in_one_revision() {
    let fixture = StateFixture::sqlite("combined-one-revision").await;
    fixture.seed_state(Some(true), false, 12).await;

    let state = fixture
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: Some(false),
                is_starred: Some(true),
            },
        )
        .await
        .expect("combined patch should remain typed")
        .expect("visible entry should update");

    assert!(!state.is_read);
    assert!(state.is_starred);
    let stored = fixture.state_row().await.unwrap();
    assert_eq!(stored.read_override, None);
    assert!(stored.is_starred);
    assert_eq!(stored.revision, 13);
}

#[tokio::test]
async fn sqlite_two_connections_merge_concurrent_different_field_patches() {
    let fixture = StateFixture::sqlite("concurrent-different-fields").await;
    let second_database = connect_for_contract(SecretString::from(fixture.url.clone())).await;
    let read_repository = fixture.repository.clone();
    let star_repository = FeedRepository::new(second_database.clone());
    let start = std::sync::Arc::new(tokio::sync::Barrier::new(3));

    let read_start = start.clone();
    let read = tokio::spawn(async move {
        read_start.wait().await;
        read_repository
            .update_state_for_user(
                USER_A_ID,
                ENTRY_A_ID,
                UpdateEntryState {
                    is_read: Some(true),
                    is_starred: None,
                },
            )
            .await
    });
    let star_start = start.clone();
    let star = tokio::spawn(async move {
        star_start.wait().await;
        star_repository
            .update_state_for_user(
                USER_A_ID,
                ENTRY_A_ID,
                UpdateEntryState {
                    is_read: None,
                    is_starred: Some(true),
                },
            )
            .await
    });
    start.wait().await;

    read.await
        .expect("read task should join")
        .expect("read patch should remain typed")
        .expect("read patch should stay visible");
    star.await
        .expect("star task should join")
        .expect("star patch should remain typed")
        .expect("star patch should stay visible");

    let stored = fixture.state_row().await.unwrap();
    assert_eq!(stored.read_override, Some(true));
    assert!(stored.is_starred);
    assert!(stored.starred_at.is_some());
    assert_eq!(stored.revision, 2);

    second_database
        .close()
        .await
        .expect("second SQLite database should close");
}

#[tokio::test]
async fn postgres_waiting_patch_observes_and_preserves_committed_field() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("postgres state lock contract skipped: test database URL is not configured");
        return;
    };
    backend_waiting_patch_preserves_committed_field(&url, DatabaseBackend::Postgres).await;
}

#[tokio::test]
async fn mysql_waiting_patch_observes_and_preserves_committed_field() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("mysql state lock contract skipped: test database URL is not configured");
        return;
    };
    backend_waiting_patch_preserves_committed_field(&url, DatabaseBackend::MySql).await;
}

async fn backend_waiting_patch_preserves_committed_field(url: &str, backend: DatabaseBackend) {
    let first_database = reset_backend_fixture(url).await;
    let second_database = connect_single_for_contract(url).await;
    let observer_database = connect_single_for_contract(url).await;
    let second_connection_id = backend_connection_id(&second_database, backend).await;
    let first_transaction = first_database
        .begin()
        .await
        .expect("first state transaction should start");
    first_transaction
        .query_one(Statement::from_sql_and_values(
            backend,
            match backend {
                DatabaseBackend::Postgres => {
                    "SELECT s.id
                     FROM subscriptions s
                     JOIN entries e ON e.feed_id = s.feed_id
                     WHERE s.user_id = $1 AND e.id = $2
                       AND e.feed_sequence > s.start_sequence
                     LIMIT 1 FOR UPDATE OF s"
                }
                DatabaseBackend::MySql => {
                    "SELECT s.id
                     FROM subscriptions s
                     JOIN entries e ON e.feed_id = s.feed_id
                     WHERE s.user_id = ? AND e.id = ?
                       AND e.feed_sequence > s.start_sequence
                     LIMIT 1 FOR UPDATE"
                }
                DatabaseBackend::Sqlite => unreachable!("backend contract is server-only"),
            },
            [USER_A_ID.into(), ENTRY_A_ID.into()],
        ))
        .await
        .expect("first transaction should lock visible subscription")
        .expect("visible subscription should exist");

    let second_repository = FeedRepository::new(second_database.clone());
    let started = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let second_started = started.clone();
    let second = tokio::spawn(async move {
        second_started.wait().await;
        second_repository
            .update_state_for_user(
                USER_A_ID,
                ENTRY_A_ID,
                UpdateEntryState {
                    is_read: Some(true),
                    is_starred: None,
                },
            )
            .await
    });
    started.wait().await;
    wait_until_backend_reports_lock_wait(&observer_database, backend, second_connection_id).await;

    first_transaction
        .execute(Statement::from_sql_and_values(
            backend,
            match backend {
                DatabaseBackend::Postgres => {
                    "INSERT INTO entry_states (
                         user_id, entry_id, feed_id, feed_sequence, read_override,
                         is_starred, starred_at, revision, updated_at
                     ) VALUES ($1, $2, $3, $4, NULL, TRUE,
                               clock_timestamp(), 1, clock_timestamp())"
                }
                DatabaseBackend::MySql => {
                    "INSERT INTO entry_states (
                         user_id, entry_id, feed_id, feed_sequence, read_override,
                         is_starred, starred_at, revision, updated_at
                     ) VALUES (?, ?, ?, ?, NULL, TRUE,
                               UTC_TIMESTAMP(6), 1, UTC_TIMESTAMP(6))"
                }
                DatabaseBackend::Sqlite => unreachable!("backend contract is server-only"),
            },
            [
                USER_A_ID.into(),
                ENTRY_A_ID.into(),
                FEED_ID.into(),
                2_i64.into(),
            ],
        ))
        .await
        .expect("first transaction should write star state");
    first_transaction
        .commit()
        .await
        .expect("first state transaction should commit");

    let second_state = tokio::time::timeout(std::time::Duration::from_secs(5), second)
        .await
        .expect("waiting state patch should resume after commit")
        .expect("waiting state task should join")
        .expect("waiting state patch should remain typed")
        .expect("waiting state patch should stay visible");
    assert!(second_state.is_read);
    assert!(second_state.is_starred);
    let stored = entry_state::Entity::find_by_id((USER_A_ID.to_owned(), ENTRY_A_ID.to_owned()))
        .one(&first_database)
        .await
        .expect("final backend state should query")
        .expect("final backend state should exist");
    assert_eq!(stored.read_override, Some(true));
    assert!(stored.is_starred);
    assert!(stored.starred_at.is_some());
    assert_eq!(stored.revision, 2);

    first_database
        .close()
        .await
        .expect("first backend database should close");
    second_database
        .close()
        .await
        .expect("second backend database should close");
    observer_database
        .close()
        .await
        .expect("observer backend database should close");
}

async fn reset_backend_fixture(url: &str) -> DatabaseConnection {
    let database = connect_for_contract(SecretString::from(url.to_owned())).await;
    rollback(&database)
        .await
        .expect("dedicated state contract database should reset");
    migrate(&database)
        .await
        .expect("state contract migrations should apply");
    insert_user(&database, USER_A_ID, "reader-a").await;
    insert_feed(&database, FIXTURE_AT).await;
    insert_subscription(&database, SUBSCRIPTION_A_ID, USER_A_ID, FIXTURE_AT).await;
    insert_entry(
        &database,
        ENTRY_A_ID,
        2,
        "entry-a",
        HASH_A,
        Some(1_784_246_400_000_000),
        FIXTURE_AT,
    )
    .await;
    database
}

async fn connect_single_for_contract(url: &str) -> DatabaseConnection {
    let mut options = ConnectOptions::new(url.to_owned());
    options
        .min_connections(1)
        .max_connections(1)
        .connect_timeout(std::time::Duration::from_secs(5))
        .acquire_timeout(std::time::Duration::from_secs(5))
        .sqlx_logging(false);
    if url.starts_with("postgres:") || url.starts_with("postgresql:") {
        options.map_sqlx_postgres_opts(|options| options.options([("timezone", "UTC")]));
    } else {
        options.map_sqlx_mysql_opts(|options| options.timezone(Some("+00:00".to_owned())));
    }
    Database::connect(options)
        .await
        .expect("single-connection state contract database should connect")
}

async fn backend_connection_id(database: &DatabaseConnection, backend: DatabaseBackend) -> i64 {
    let row = database
        .query_one(Statement::from_string(
            backend,
            match backend {
                DatabaseBackend::Postgres => "SELECT pg_backend_pid() AS connection_id",
                DatabaseBackend::MySql => "SELECT CAST(CONNECTION_ID() AS SIGNED) AS connection_id",
                DatabaseBackend::Sqlite => unreachable!("backend contract is server-only"),
            }
            .to_owned(),
        ))
        .await
        .expect("backend connection id should query")
        .expect("backend connection id should exist");
    match backend {
        DatabaseBackend::Postgres => i64::from(
            row.try_get::<i32>("", "connection_id")
                .expect("PostgreSQL backend PID should decode"),
        ),
        DatabaseBackend::MySql => row
            .try_get::<i64>("", "connection_id")
            .expect("MySQL connection ID should decode"),
        DatabaseBackend::Sqlite => unreachable!("backend contract is server-only"),
    }
}

async fn wait_until_backend_reports_lock_wait(
    observer: &DatabaseConnection,
    backend: DatabaseBackend,
    connection_id: i64,
) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if backend_reports_lock_wait(observer, backend, connection_id).await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("second patch must visibly wait on the subscription lock");
}

async fn backend_reports_lock_wait(
    observer: &DatabaseConnection,
    backend: DatabaseBackend,
    connection_id: i64,
) -> bool {
    match backend {
        DatabaseBackend::Postgres => observer
            .query_one(Statement::from_sql_and_values(
                backend,
                "SELECT EXISTS (
                     SELECT 1 FROM pg_stat_activity
                     WHERE pid = $1 AND state = 'active'
                       AND wait_event_type = 'Lock'
                 ) AS waiting",
                [connection_id.into()],
            ))
            .await
            .expect("PostgreSQL lock wait should query")
            .expect("PostgreSQL lock wait result should exist")
            .try_get::<bool>("", "waiting")
            .expect("PostgreSQL lock wait should decode"),
        DatabaseBackend::MySql => observer
            .query_all(Statement::from_string(
                backend,
                "SHOW PROCESSLIST".to_owned(),
            ))
            .await
            .expect("MySQL process list should query without PROCESS privilege")
            .iter()
            .find(|row| mysql_processlist_id(row) == connection_id)
            .is_some_and(mysql_processlist_row_is_lock_wait),
        DatabaseBackend::Sqlite => unreachable!("backend contract is server-only"),
    }
}

fn mysql_processlist_id(row: &QueryResult) -> i64 {
    for column in ["Id", "ID", "id"] {
        if let Ok(value) = row.try_get::<i64>("", column) {
            return value;
        }
        if let Ok(value) = row.try_get::<u64>("", column) {
            return i64::try_from(value).expect("MySQL process ID should fit signed 64-bit");
        }
        if let Ok(value) = row.try_get::<u32>("", column) {
            return i64::from(value);
        }
    }
    panic!("MySQL SHOW PROCESSLIST ID column should decode");
}

fn mysql_processlist_row_is_lock_wait(row: &QueryResult) -> bool {
    let command = mysql_processlist_text(row, &["Command", "COMMAND", "command"])
        .expect("MySQL SHOW PROCESSLIST Command should be non-null");
    let state = mysql_processlist_text(row, &["State", "STATE", "state"]);
    let command = command.trim().to_ascii_lowercase();
    matches!(command.as_str(), "query" | "execute")
        && state.is_some_and(|state| state.to_ascii_lowercase().contains("lock"))
}

fn mysql_processlist_text(row: &QueryResult, columns: &[&str]) -> Option<String> {
    for column in columns {
        if let Ok(value) = row.try_get::<Option<String>>("", column) {
            return value;
        }
        if let Ok(value) = row.try_get::<String>("", column) {
            return Some(value);
        }
    }
    None
}

#[tokio::test]
async fn sqlite_read_patch_uses_sparse_override_and_preserves_star() {
    let fixture = StateFixture::sqlite("read-preserves-star").await;
    fixture.seed_state(Some(false), true, 1).await;

    let state = fixture
        .repository
        .update_state_for_user(
            USER_A_ID,
            ENTRY_A_ID,
            UpdateEntryState {
                is_read: Some(true),
                is_starred: None,
            },
        )
        .await
        .expect("state update should remain typed")
        .expect("visible entry should update");

    assert!(state.is_read);
    assert!(state.is_starred);
    let stored = fixture.state_row().await.unwrap();
    assert_eq!(stored.read_override, Some(true));
    assert!(stored.is_starred);
    assert_eq!(stored.revision, 2);
}
