#[allow(dead_code)]
mod support;

use std::time::Duration;

use raindrop::{
    db::{
        entities::{entry, feed, feed_refresh_run, lifecycle_outbox, rss_counter},
        migrate, rollback,
    },
    feeds::{
        ClaimRequest, FeedParser, FeedRepository, FeedUrlPolicy, FetchOutcome, FetchedDocument,
        PersistFeed, QueueRefreshRequest, RefreshCounts, RefreshFailure, RefreshRepositoryError,
        RefreshStatus, RefreshTrigger,
    },
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend,
    DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Statement,
};
use secrecy::SecretString;
use support::database::{FEED_ID, connect_for_contract, insert_feed};

static PARSER_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn sqlite_lifecycle_outbox_contract() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("lifecycle-contract.db").display()
    );
    backend_lifecycle_outbox_contract(&url).await;
}

#[tokio::test]
async fn postgres_lifecycle_outbox_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!(
            "postgres lifecycle outbox contract skipped: test database URL is not configured"
        );
        return;
    };
    backend_lifecycle_outbox_contract(&url).await;
}

#[tokio::test]
async fn mysql_lifecycle_outbox_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("mysql lifecycle outbox contract skipped: test database URL is not configured");
        return;
    };
    backend_lifecycle_outbox_contract(&url).await;
}

#[tokio::test]
async fn sqlite_event_failure_rolls_back_the_whole_feed_persist() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("lifecycle-rollback.db").display()
    );
    let database = reset_contract_database(&url).await;
    let repository = FeedRepository::new(database.clone());
    let claim = claim_refresh(&repository, "rollback").await;
    database
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "CREATE TRIGGER fail_completed_lifecycle_event BEFORE INSERT ON lifecycle_outbox
             WHEN NEW.event_sequence = 20
             BEGIN SELECT RAISE(ABORT, 'induced completed event failure'); END"
                .to_owned(),
        ))
        .await
        .expect("lifecycle failure trigger should create");

    let result = repository
        .persist_feed(
            &claim,
            PersistFeed::try_from(parsed_feed(single_item_feed("rollback")).await)
                .expect("rollback input should map"),
        )
        .await;
    assert!(matches!(result, Err(RefreshRepositoryError::Database(_))));
    assert_persist_rolled_back(&database, &claim, 0).await;

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_record_lease_lost_never_records_a_lifecycle_event() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("lifecycle-lease-lost.db").display()
    );
    let database = reset_contract_database(&url).await;
    let repository = FeedRepository::new(database.clone());
    let claim =
        claim_refresh_with_lease(&repository, "lease-lost", Duration::from_millis(40)).await;

    tokio::time::sleep(Duration::from_millis(80)).await;
    repository
        .record_lease_lost(&claim.run_id)
        .await
        .expect("expired run should record lease loss");

    assert!(lifecycle_events(&database).await.is_empty());
    let run = feed_refresh_run::Entity::find_by_id(&claim.run_id)
        .one(&database)
        .await
        .expect("lease-lost run should query")
        .expect("lease-lost run should exist");
    assert_eq!(run.status, RefreshStatus::LeaseLost.as_str());

    database.close().await.expect("database should close");
}

async fn backend_lifecycle_outbox_contract(url: &str) {
    successful_200_contract(url).await;
    partial_200_contract(url).await;
    not_modified_contract(url).await;
    owned_error_contract(url).await;
    legacy_completion_contract(url).await;
    stale_completion_contract(url).await;
    exact_duplicate_retry_contract(url).await;
    semantic_conflict_contract(url).await;
    order_conflict_contract(url).await;
    oversized_error_code_contract(url).await;
}

async fn successful_200_contract(url: &str) {
    let database = reset_contract_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let claim = claim_refresh(&repository, "success-200").await;
    let result = repository
        .persist_feed(
            &claim,
            PersistFeed::try_from(parsed_feed(single_item_feed("success")).await)
                .expect("success input should map"),
        )
        .await
        .expect("successful feed should persist with lifecycle events");
    assert_eq!(result.counts, counts(1, 0, 0));
    assert_eq!(result.generation, Some(1));

    let events = lifecycle_events(&database).await;
    assert_eq!(events.len(), 2);
    assert_event(
        &events[0],
        &claim,
        "feed.refresh.persisted",
        10,
        &format!(
            "{{\"eventType\":\"feed.refresh.persisted\",\"payloadVersion\":1,\"refreshId\":\"{}\",\"feedId\":\"{}\",\"commitGeneration\":1,\"newCount\":1,\"updatedCount\":0,\"droppedCount\":0}}",
            claim.run_id, claim.feed_id
        ),
        &format!("refresh:{}:persisted:v1", claim.run_id),
    );
    assert_event(
        &events[1],
        &claim,
        "feed.refresh.completed",
        20,
        &completed_payload(&claim, "SUCCESS", Some(200), counts(1, 0, 0), None),
        &format!("refresh:{}:completed:v1", claim.run_id),
    );

    database.close().await.expect("database should close");
}

async fn partial_200_contract(url: &str) {
    let database = reset_contract_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let claim = claim_refresh(&repository, "partial-200").await;
    let parsed = parsed_feed(duplicate_item_feed()).await;
    assert_eq!(parsed.duplicate_count(), 1);
    let result = repository
        .persist_feed(
            &claim,
            PersistFeed::try_from(parsed).expect("partial input should map"),
        )
        .await
        .expect("partially deduplicated feed should persist");
    assert_eq!(result.counts, counts(1, 0, 1));
    assert_eq!(result.generation, Some(1));

    let events = lifecycle_events(&database).await;
    assert_eq!(events.len(), 2);
    assert_event(
        &events[0],
        &claim,
        "feed.refresh.persisted",
        10,
        &format!(
            "{{\"eventType\":\"feed.refresh.persisted\",\"payloadVersion\":1,\"refreshId\":\"{}\",\"feedId\":\"{}\",\"commitGeneration\":1,\"newCount\":1,\"updatedCount\":0,\"droppedCount\":1}}",
            claim.run_id, claim.feed_id
        ),
        &format!("refresh:{}:persisted:v1", claim.run_id),
    );
    assert_event(
        &events[1],
        &claim,
        "feed.refresh.completed",
        20,
        &completed_payload(&claim, "PARTIAL", Some(200), counts(1, 0, 1), None),
        &format!("refresh:{}:completed:v1", claim.run_id),
    );

    database.close().await.expect("database should close");
}

async fn not_modified_contract(url: &str) {
    let database = reset_contract_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let claim = claim_refresh(&repository, "not-modified").await;

    repository
        .complete_not_modified(&claim)
        .await
        .expect("owned 304 should complete");

    let events = lifecycle_events(&database).await;
    assert_eq!(events.len(), 1);
    assert_event(
        &events[0],
        &claim,
        "feed.refresh.completed",
        20,
        &completed_payload(&claim, "NOT_MODIFIED", Some(304), counts(0, 0, 0), None),
        &format!("refresh:{}:completed:v1", claim.run_id),
    );

    database.close().await.expect("database should close");
}

async fn owned_error_contract(url: &str) {
    let database = reset_contract_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let claim = claim_refresh(&repository, "owned-error").await;

    repository
        .complete_failure(
            &claim,
            RefreshFailure {
                error_code: "FETCH_FAILED".to_owned(),
                http_status: Some(503),
                retry_at: Some(time::OffsetDateTime::now_utc() + time::Duration::minutes(5)),
            },
        )
        .await
        .expect("owned error should complete");

    let events = lifecycle_events(&database).await;
    assert_eq!(events.len(), 1);
    assert_event(
        &events[0],
        &claim,
        "feed.refresh.completed",
        20,
        &completed_payload(
            &claim,
            "ERROR",
            Some(503),
            counts(0, 0, 0),
            Some("FETCH_FAILED"),
        ),
        &format!("refresh:{}:completed:v1", claim.run_id),
    );
    assert!(!events[0].payload_json.contains("retry"));

    database.close().await.expect("database should close");
}

async fn legacy_completion_contract(url: &str) {
    let database = reset_contract_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let success_claim = claim_refresh(&repository, "legacy-success").await;
    repository
        .complete_success(&success_claim, 200, counts(3, 2, 0))
        .await
        .expect("legacy success should complete");
    let success_events = lifecycle_events(&database).await;
    assert_eq!(success_events.len(), 1);
    assert_event(
        &success_events[0],
        &success_claim,
        "feed.refresh.completed",
        20,
        &completed_payload(&success_claim, "SUCCESS", Some(200), counts(3, 2, 0), None),
        &format!("refresh:{}:completed:v1", success_claim.run_id),
    );
    database.close().await.expect("database should close");

    let database = reset_contract_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let partial_claim = claim_refresh(&repository, "legacy-partial").await;
    repository
        .complete_partial(&partial_claim, 200, counts(2, 1, 3))
        .await
        .expect("legacy partial should complete");
    let partial_events = lifecycle_events(&database).await;
    assert_eq!(partial_events.len(), 1);
    assert_event(
        &partial_events[0],
        &partial_claim,
        "feed.refresh.completed",
        20,
        &completed_payload(&partial_claim, "PARTIAL", Some(200), counts(2, 1, 3), None),
        &format!("refresh:{}:completed:v1", partial_claim.run_id),
    );
    database.close().await.expect("database should close");
}

async fn stale_completion_contract(url: &str) {
    let database = reset_contract_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let claim = claim_refresh(&repository, "stale-completion").await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.lease_token = Set(claim.lease_token + 1);
    active
        .update(&database)
        .await
        .expect("newer lease token should persist");

    let result = repository
        .complete_failure(
            &claim,
            RefreshFailure {
                error_code: "FETCH_FAILED".to_owned(),
                http_status: None,
                retry_at: None,
            },
        )
        .await;
    assert!(matches!(result, Err(RefreshRepositoryError::LeaseLost)));
    assert!(lifecycle_events(&database).await.is_empty());

    database.close().await.expect("database should close");
}

async fn exact_duplicate_retry_contract(url: &str) {
    let database = reset_contract_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let claim = claim_refresh(&repository, "exact-retry").await;
    let persisted_payload = format!(
        "{{\"eventType\":\"feed.refresh.persisted\",\"payloadVersion\":1,\"refreshId\":\"{}\",\"feedId\":\"{}\",\"commitGeneration\":1,\"newCount\":1,\"updatedCount\":0,\"droppedCount\":0}}",
        claim.run_id, claim.feed_id
    );
    let completed_payload = completed_payload(&claim, "SUCCESS", Some(200), counts(1, 0, 0), None);
    insert_existing_event(
        &database,
        &claim,
        "feed.refresh.persisted",
        10,
        &persisted_payload,
        &format!("refresh:{}:persisted:v1", claim.run_id),
    )
    .await;
    insert_existing_event(
        &database,
        &claim,
        "feed.refresh.completed",
        20,
        &completed_payload,
        &format!("refresh:{}:completed:v1", claim.run_id),
    )
    .await;

    repository
        .persist_feed(
            &claim,
            PersistFeed::try_from(parsed_feed(single_item_feed("retry")).await)
                .expect("retry input should map"),
        )
        .await
        .expect("exact existing lifecycle events should be idempotent");
    assert_eq!(lifecycle_events(&database).await.len(), 2);

    database.close().await.expect("database should close");
}

async fn semantic_conflict_contract(url: &str) {
    let database = reset_contract_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let claim = claim_refresh(&repository, "semantic-conflict").await;
    insert_existing_event(
        &database,
        &claim,
        "feed.refresh.persisted",
        10,
        "{\"conflict\":true}",
        &format!("refresh:{}:persisted:v1", claim.run_id),
    )
    .await;

    let result = repository
        .persist_feed(
            &claim,
            PersistFeed::try_from(parsed_feed(single_item_feed("semantic-conflict")).await)
                .expect("conflicting input should map"),
        )
        .await;
    assert!(matches!(
        result,
        Err(RefreshRepositoryError::LifecycleEventConflict)
    ));
    assert_persist_rolled_back(&database, &claim, 1).await;

    database.close().await.expect("database should close");
}

async fn order_conflict_contract(url: &str) {
    let database = reset_contract_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let claim = claim_refresh(&repository, "order-conflict").await;
    insert_existing_event(
        &database,
        &claim,
        "feed.refresh.persisted",
        10,
        "{\"orderConflict\":true}",
        "refresh:other:persisted:v1",
    )
    .await;

    let result = repository
        .persist_feed(
            &claim,
            PersistFeed::try_from(parsed_feed(single_item_feed("order-conflict")).await)
                .expect("order-conflicting input should map"),
        )
        .await;
    assert!(matches!(
        result,
        Err(RefreshRepositoryError::LifecycleEventConflict)
    ));
    assert_persist_rolled_back(&database, &claim, 1).await;

    database.close().await.expect("database should close");
}

async fn oversized_error_code_contract(url: &str) {
    let database = reset_contract_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let claim = claim_refresh(&repository, "oversized-error").await;
    let result = repository
        .complete_failure(
            &claim,
            RefreshFailure {
                error_code: "X".repeat(64 * 1024),
                http_status: None,
                retry_at: None,
            },
        )
        .await;
    assert!(matches!(
        result,
        Err(RefreshRepositoryError::InvalidRequest)
    ));
    assert!(lifecycle_events(&database).await.is_empty());
    let run = feed_refresh_run::Entity::find_by_id(&claim.run_id)
        .one(&database)
        .await
        .expect("oversized error run should query")
        .expect("oversized error run should exist");
    assert_eq!(run.status, RefreshStatus::Running.as_str());

    database.close().await.expect("database should close");
}

async fn reset_contract_database(url: &str) -> DatabaseConnection {
    let database = connect_for_contract(SecretString::from(url.to_owned())).await;
    rollback(&database)
        .await
        .unwrap_or_else(|_| panic!("dedicated lifecycle contract database should reset"));
    migrate(&database)
        .await
        .expect("lifecycle migrations should apply");
    insert_feed(&database, time::OffsetDateTime::now_utc()).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.entry_sequence_head = Set(0);
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.orphaned_at = Set(None);
    active.update(&database).await.expect("feed should unlock");
    database
}

async fn claim_refresh(repository: &FeedRepository, key: &str) -> raindrop::feeds::RefreshClaim {
    claim_refresh_with_lease(repository, key, Duration::from_secs(30)).await
}

async fn claim_refresh_with_lease(
    repository: &FeedRepository,
    key: &str,
    lease_duration: Duration,
) -> raindrop::feeds::RefreshClaim {
    repository
        .queue_refresh(QueueRefreshRequest {
            feed_id: FEED_ID.to_owned(),
            requested_by_user_id: None,
            trigger: RefreshTrigger::Manual,
            idempotency_key: format!("lifecycle:{key}"),
        })
        .await
        .expect("refresh should queue");
    repository
        .claim_due(ClaimRequest {
            owner: format!("worker-{key}"),
            lease_duration,
        })
        .await
        .expect("refresh claim should not fail")
        .expect("refresh should claim")
}

async fn parsed_feed(body: Vec<u8>) -> raindrop::feeds::ParsedFeed {
    let _guard = PARSER_MUTEX.lock().await;
    let url = FeedUrlPolicy::new(true)
        .normalize("https://feeds.example.test/final.xml?secret=must-not-leak")
        .expect("fixture URL should normalize");
    let document = FetchedDocument::try_from(FetchOutcome::Document {
        url,
        document: body,
        content_type: Some("application/rss+xml".to_owned()),
        etag: None,
        last_modified: None,
    })
    .expect("fixture should be a fetched document");
    FeedParser::new()
        .parse(document)
        .await
        .expect("fixture should parse")
}

fn single_item_feed(guid: &str) -> Vec<u8> {
    format!(
        "<rss version=\"2.0\"><channel><title>x</title><link>https://example.test/</link>
         <item><guid>{guid}</guid><title>Lifecycle</title><description><![CDATA[<p>publisher-secret-body</p>]]></description></item>
         </channel></rss>"
    )
    .into_bytes()
}

fn duplicate_item_feed() -> Vec<u8> {
    br#"<rss version="2.0"><channel><title>x</title><link>https://example.test/</link>
        <item><guid>duplicate-guid</guid><title>First</title><description>one</description></item>
        <item><guid>duplicate-guid</guid><title>Second</title><description>two</description></item>
        </channel></rss>"#
        .to_vec()
}

fn counts(new_count: i32, updated_count: i32, dropped_count: i32) -> RefreshCounts {
    RefreshCounts {
        new_count,
        updated_count,
        dropped_count,
    }
}

fn completed_payload(
    claim: &raindrop::feeds::RefreshClaim,
    status: &str,
    http_status: Option<i32>,
    counts: RefreshCounts,
    error_code: Option<&str>,
) -> String {
    format!(
        "{{\"eventType\":\"feed.refresh.completed\",\"payloadVersion\":1,\"refreshId\":\"{}\",\"feedId\":\"{}\",\"status\":\"{}\",\"httpStatus\":{},\"newCount\":{},\"updatedCount\":{},\"droppedCount\":{},\"errorCode\":{}}}",
        claim.run_id,
        claim.feed_id,
        status,
        http_status.map_or_else(|| "null".to_owned(), |value| value.to_string()),
        counts.new_count,
        counts.updated_count,
        counts.dropped_count,
        error_code.map_or_else(
            || "null".to_owned(),
            |value| serde_json::to_string(value).expect("error code should serialize")
        )
    )
}

async fn lifecycle_events(database: &DatabaseConnection) -> Vec<lifecycle_outbox::Model> {
    lifecycle_outbox::Entity::find()
        .order_by_asc(lifecycle_outbox::Column::EventSequence)
        .all(database)
        .await
        .expect("lifecycle events should query")
}

fn assert_event(
    event: &lifecycle_outbox::Model,
    claim: &raindrop::feeds::RefreshClaim,
    event_type: &str,
    sequence: i32,
    payload: &str,
    idempotency_key: &str,
) {
    assert_eq!(event.event_type, event_type);
    assert_eq!(event.aggregate_type, "FEED");
    assert_eq!(event.aggregate_id, claim.feed_id);
    assert_eq!(event.refresh_id, claim.run_id);
    assert_eq!(event.event_sequence, sequence);
    assert_eq!(event.payload_version, 1);
    assert_eq!(event.payload_json.as_bytes(), payload.as_bytes());
    assert!(event.payload_json.len() <= 64 * 1024);
    assert!(!event.payload_json.contains("publisher-secret"));
    assert!(!event.payload_json.contains("must-not-leak"));
    assert_eq!(event.idempotency_key, idempotency_key);
    assert_eq!(event.status, "PENDING");
    assert_eq!(event.attempts, 0);
    assert_eq!(event.lease_owner, None);
    assert_eq!(event.lease_until, None);
    assert_eq!(event.completed_at, None);
    assert!(event.created_at >= event.available_at);
}

async fn insert_existing_event(
    database: &DatabaseConnection,
    claim: &raindrop::feeds::RefreshClaim,
    event_type: &str,
    sequence: i32,
    payload: &str,
    idempotency_key: &str,
) {
    let backend = database.get_database_backend();
    let sql = match backend {
        DatabaseBackend::Sqlite => {
            "INSERT INTO lifecycle_outbox (
                id,event_type,aggregate_type,aggregate_id,refresh_id,event_sequence,payload_version,
                payload_json,idempotency_key,available_at,created_at
             ) VALUES (?,?,?,?,?,?,?,?,?,
                strftime('%Y-%m-%dT%H:%M:%f000Z','now'),
                strftime('%Y-%m-%dT%H:%M:%f000Z','now'))"
        }
        DatabaseBackend::Postgres => {
            "INSERT INTO lifecycle_outbox (
                id,event_type,aggregate_type,aggregate_id,refresh_id,event_sequence,payload_version,
                payload_json,idempotency_key,available_at,created_at
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,clock_timestamp(),clock_timestamp())"
        }
        DatabaseBackend::MySql => {
            "INSERT INTO lifecycle_outbox (
                id,event_type,aggregate_type,aggregate_id,refresh_id,event_sequence,payload_version,
                payload_json,idempotency_key,available_at,created_at
             ) VALUES (?,?,?,?,?,?,?,?,?,UTC_TIMESTAMP(6),UTC_TIMESTAMP(6))"
        }
    };
    database
        .execute(Statement::from_sql_and_values(
            backend,
            sql,
            [
                uuid::Uuid::new_v4().to_string().into(),
                event_type.into(),
                "FEED".into(),
                claim.feed_id.as_str().into(),
                claim.run_id.as_str().into(),
                sequence.into(),
                1.into(),
                payload.into(),
                idempotency_key.into(),
            ],
        ))
        .await
        .expect("existing lifecycle event should insert");
}

async fn assert_persist_rolled_back(
    database: &DatabaseConnection,
    claim: &raindrop::feeds::RefreshClaim,
    expected_event_count: usize,
) {
    assert!(
        entry::Entity::find()
            .filter(entry::Column::FeedId.eq(FEED_ID))
            .all(database)
            .await
            .expect("rolled back entries should query")
            .is_empty()
    );
    assert_eq!(
        rss_counter::Entity::find_by_id("INGEST_GENERATION")
            .one(database)
            .await
            .expect("generation should query")
            .expect("generation should exist")
            .value,
        0
    );
    let feed = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("rolled back feed should query")
        .expect("rolled back feed should exist");
    assert_eq!(feed.entry_sequence_head, 0);
    assert_eq!(feed.title, None);
    assert_eq!(feed.site_url, None);
    assert_eq!(feed.lease_owner.as_deref(), Some(claim.owner.as_str()));
    assert_eq!(feed.lease_token, claim.lease_token);
    assert!(feed.lease_until.is_some());
    let run = feed_refresh_run::Entity::find_by_id(&claim.run_id)
        .one(database)
        .await
        .expect("rolled back run should query")
        .expect("rolled back run should exist");
    assert_eq!(run.status, RefreshStatus::Running.as_str());
    assert_eq!(run.commit_generation, None);
    assert_eq!(
        (run.new_count, run.updated_count, run.dropped_count),
        (0, 0, 0)
    );
    assert_eq!(lifecycle_events(database).await.len(), expected_event_count);
}
