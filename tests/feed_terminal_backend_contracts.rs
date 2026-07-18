#[allow(dead_code)]
mod support;

use std::time::Duration;

use raindrop::db::{
    entities::{entry, feed, feed_refresh_run, lifecycle_outbox},
    migrate, rollback,
};
use raindrop::feeds::{
    ClaimRequest, ExactClaimResult, FeedParser, FeedRepository, FeedUrlPolicy, FetchOutcome,
    FetchedDocument, PersistFeed, QueueRefreshRequest, RefreshFailure, RefreshRepositoryError,
    RefreshResult, RefreshSchedule, RefreshStatus, RefreshTrigger,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, EntityTrait, IntoActiveModel,
    PaginatorTrait, Statement, TransactionTrait,
};
use secrecy::SecretString;

use support::database::{FEED_ID, connect_for_contract, insert_feed};

struct ZeroJitter;

impl raindrop::feeds::JitterSource for ZeroJitter {
    fn sample_inclusive_us(&mut self, _upper_bound_us: u64) -> u64 {
        0
    }
}

#[tokio::test]
async fn postgres_terminal_atomicity_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL terminal atomicity; RAINDROP_TEST_POSTGRES_URL is unset");
        return;
    };
    terminal_contract(SecretString::from(url), "postgres").await;
}

#[tokio::test]
async fn mysql_terminal_atomicity_and_feed_first_lock_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("skipping MySQL terminal atomicity; RAINDROP_TEST_MYSQL_URL is unset");
        return;
    };
    let url = SecretString::from(url);
    terminal_contract(url.clone(), "mysql").await;
    mysql_feed_first_lock_contract(url).await;
}

async fn terminal_contract(url: SecretString, backend_name: &str) {
    let database = reset_database(url).await;
    let repository = FeedRepository::new(database.clone());

    let older = queue(&repository, "older").await;
    let mut older_fixture = feed_refresh_run::Entity::find_by_id(&older.id)
        .one(&database)
        .await
        .unwrap()
        .unwrap()
        .into_active_model();
    older_fixture.status = Set("SUCCESS".to_owned());
    older_fixture.completed_at = Set(Some(time::OffsetDateTime::now_utc()));
    older_fixture.update(&database).await.unwrap();
    let target = queue(&repository, "target").await;
    let claim = claimed(
        repository
            .claim_run(&target.id, claim_request("target-worker"))
            .await
            .expect("exact target claim"),
    );
    assert_eq!(claim.run_id, target.id);
    assert_eq!(
        feed_refresh_run::Entity::find_by_id(older.id)
            .one(&database)
            .await
            .unwrap()
            .unwrap()
            .status,
        "SUCCESS"
    );

    let parsed = FeedParser::new()
        .parse(
            FetchedDocument::try_from(FetchOutcome::Document {
                url: FeedUrlPolicy::new(false)
                    .normalize("https://example.com/feed.xml")
                    .unwrap(),
                document: b"<rss version=\"2.0\"><channel><title>x</title><link>https://example.com/</link><item><guid>backend-one</guid><description>safe</description></item></channel></rss>".to_vec(),
                content_type: Some("application/rss+xml".to_owned()),
                etag: None,
                last_modified: None,
            })
            .unwrap(),
        )
        .await
        .unwrap();
    repository
        .persist_feed_scheduled(
            &claim,
            PersistFeed::try_from(parsed).unwrap(),
            success_schedule(),
        )
        .await
        .expect("scheduled 200 commits");
    assert_eq!(entry::Entity::find().count(&database).await.unwrap(), 1);
    assert!(matches!(
        repository
            .claim_run(&target.id, claim_request("terminal-probe"))
            .await
            .unwrap(),
        ExactClaimResult::Existing(RefreshStatus::Success)
    ));
    assert!(matches!(
        repository
            .claim_run(
                "00000000-0000-4000-8000-000000000999",
                claim_request("missing-probe")
            )
            .await,
        Err(RefreshRepositoryError::RunNotFound)
    ));

    let not_modified = queue(&repository, "not-modified").await;
    let claim = claimed(
        repository
            .claim_run(&not_modified.id, claim_request("not-modified-worker"))
            .await
            .unwrap(),
    );
    let redirected = FeedUrlPolicy::new(false)
        .normalize("https://redirected.example.test/feed.xml")
        .unwrap();
    repository
        .complete_not_modified_scheduled(&claim, &redirected, None, None, success_schedule())
        .await
        .expect("scheduled 304 commits");
    let after_304 = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after_304.validator_url.as_deref(),
        Some("https://redirected.example.test/feed.xml")
    );
    assert!(after_304.etag.is_none() && after_304.last_modified.is_none());

    let failed = queue(&repository, "failure").await;
    let claim = claimed(
        repository
            .claim_run(&failed.id, claim_request("failure-worker"))
            .await
            .unwrap(),
    );
    let schedule = RefreshSchedule::new(ZeroJitter)
        .after_result(
            time::OffsetDateTime::now_utc(),
            0,
            RefreshResult::TransientFailure { retry_after: None },
        )
        .unwrap();
    repository
        .complete_failure_scheduled(
            &claim,
            RefreshFailure {
                error_code: "FETCH_FAILED".to_owned(),
                http_status: None,
                retry_at: Some(schedule.next_at()),
            },
            schedule,
        )
        .await
        .expect("scheduled error commits");
    let after_error = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_error.consecutive_failures, 1);
    assert_eq!(after_error.last_error_code.as_deref(), Some("FETCH_FAILED"));
    assert_eq!(
        lifecycle_outbox::Entity::find()
            .count(&database)
            .await
            .unwrap(),
        4
    );
    let disabled = queue(&repository, "disabled").await;
    feed::Entity::update_many()
        .col_expr(feed::Column::IsDisabled, true.into())
        .exec(&database)
        .await
        .unwrap();
    assert!(matches!(
        repository
            .claim_run(&disabled.id, claim_request("disabled-probe"))
            .await
            .unwrap(),
        ExactClaimResult::FeedDisabled
    ));
    feed::Entity::update_many()
        .col_expr(feed::Column::IsDisabled, false.into())
        .exec(&database)
        .await
        .unwrap();

    let rollback = queue(&repository, "rollback").await;
    let rollback_claim = claimed(
        repository
            .claim_run(&rollback.id, claim_request("rollback-worker"))
            .await
            .unwrap(),
    );
    insert_completed_conflict(&database, &rollback_claim).await;
    let before_feed = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let before_run = feed_refresh_run::Entity::find_by_id(&rollback.id)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let before_entries = entry::Entity::find().count(&database).await.unwrap();
    let before_outbox = lifecycle_outbox::Entity::find()
        .count(&database)
        .await
        .unwrap();
    let parsed = FeedParser::new().parse(FetchedDocument::try_from(FetchOutcome::Document {
        url: FeedUrlPolicy::new(false).normalize("https://redirected.example.test/feed.xml").unwrap(),
        document: b"<rss version=\"2.0\"><channel><title>x</title><link>https://example.com/</link><item><guid>rollback-new</guid><description>safe</description></item></channel></rss>".to_vec(),
        content_type: Some("application/rss+xml".to_owned()), etag: None, last_modified: None,
    }).unwrap()).await.unwrap();
    assert!(matches!(
        repository
            .persist_feed_scheduled(
                &rollback_claim,
                PersistFeed::try_from(parsed).unwrap(),
                success_schedule()
            )
            .await,
        Err(RefreshRepositoryError::LifecycleEventConflict)
    ));
    assert_eq!(
        feed::Entity::find_by_id(FEED_ID)
            .one(&database)
            .await
            .unwrap()
            .unwrap(),
        before_feed
    );
    assert_eq!(
        feed_refresh_run::Entity::find_by_id(rollback.id)
            .one(&database)
            .await
            .unwrap()
            .unwrap(),
        before_run
    );
    assert_eq!(
        entry::Entity::find().count(&database).await.unwrap(),
        before_entries
    );
    assert_eq!(
        lifecycle_outbox::Entity::find()
            .count(&database)
            .await
            .unwrap(),
        before_outbox
    );

    let stale = queue(&repository, "stale").await;
    let stale_claim = claimed(
        repository
            .claim_run(&stale.id, claim_request("stale-worker"))
            .await
            .unwrap(),
    );
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let mut active: feed::ActiveModel = model.into();
    active.lease_token = Set(stale_claim.lease_token + 1);
    active.update(&database).await.unwrap();
    let before_feed = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let before_run = feed_refresh_run::Entity::find_by_id(&stale.id)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let before_outbox = lifecycle_outbox::Entity::find()
        .count(&database)
        .await
        .unwrap();
    assert!(matches!(
        repository
            .complete_not_modified_scheduled(
                &stale_claim,
                &redirected,
                None,
                None,
                success_schedule(),
            )
            .await,
        Err(RefreshRepositoryError::LeaseLost)
    ));
    assert_eq!(
        feed::Entity::find_by_id(FEED_ID)
            .one(&database)
            .await
            .unwrap()
            .unwrap(),
        before_feed
    );
    assert_eq!(
        feed_refresh_run::Entity::find_by_id(stale.id)
            .one(&database)
            .await
            .unwrap()
            .unwrap(),
        before_run
    );
    assert_eq!(
        lifecycle_outbox::Entity::find()
            .count(&database)
            .await
            .unwrap(),
        before_outbox
    );
    database.close().await.expect("database closes");
    eprintln!("{backend_name} terminal atomicity contract passed");
}

async fn mysql_feed_first_lock_contract(url: SecretString) {
    let database = reset_database(url).await;
    let repository = FeedRepository::new(database.clone());
    let run = queue(&repository, "mysql-lock-order").await;
    let run_lock = database.begin().await.unwrap();
    run_lock
        .query_one(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::MySql,
            "SELECT id FROM feed_refresh_runs WHERE id = ? FOR UPDATE",
            [run.id.as_str().into()],
        ))
        .await
        .unwrap();
    let claim_task = tokio::spawn({
        let repository = repository.clone();
        let run_id = run.id.clone();
        async move {
            repository
                .claim_run(&run_id, claim_request("mysql-lock-worker"))
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut feed_probe = tokio::spawn({
        let database = database.clone();
        async move {
            let transaction = database.begin().await.unwrap();
            transaction
                .query_one(Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::MySql,
                    "SELECT lease_token FROM feeds WHERE id = ? FOR UPDATE",
                    [FEED_ID.into()],
                ))
                .await
                .unwrap();
            transaction.commit().await.unwrap();
        }
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut feed_probe)
            .await
            .is_err()
    );
    run_lock.commit().await.unwrap();
    assert!(matches!(
        claim_task.await.unwrap().unwrap(),
        ExactClaimResult::Claimed(_)
    ));
    feed_probe.await.unwrap();
    database.close().await.expect("database closes");
}

async fn reset_database(url: SecretString) -> sea_orm::DatabaseConnection {
    let database = connect_for_contract(url).await;
    let _ = rollback(&database).await;
    migrate(&database).await.expect("migrations apply");
    insert_feed(&database, time::OffsetDateTime::now_utc()).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .unwrap()
        .unwrap();
    let mut active: feed::ActiveModel = model.into();
    active.entry_sequence_head = Set(0);
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.orphaned_at = Set(None);
    active.update(&database).await.unwrap();
    database
}

async fn queue(repository: &FeedRepository, key: &str) -> raindrop::feeds::RefreshRun {
    repository
        .queue_refresh(QueueRefreshRequest {
            feed_id: FEED_ID.to_owned(),
            requested_by_user_id: None,
            trigger: RefreshTrigger::Manual,
            idempotency_key: key.to_owned(),
        })
        .await
        .unwrap()
}

fn claim_request(owner: &str) -> ClaimRequest {
    ClaimRequest {
        owner: owner.to_owned(),
        lease_duration: Duration::from_secs(30),
    }
}

fn claimed(result: ExactClaimResult) -> raindrop::feeds::RefreshClaim {
    let ExactClaimResult::Claimed(claim) = result else {
        panic!("run should claim exactly");
    };
    claim
}

fn success_schedule() -> raindrop::feeds::ScheduleOutcome {
    RefreshSchedule::new(ZeroJitter)
        .after_result(time::OffsetDateTime::now_utc(), 0, RefreshResult::Success)
        .unwrap()
}

async fn insert_completed_conflict(
    database: &sea_orm::DatabaseConnection,
    claim: &raindrop::feeds::RefreshClaim,
) {
    let now = time::OffsetDateTime::now_utc();
    lifecycle_outbox::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        event_type: Set("feed.refresh.completed".to_owned()),
        aggregate_type: Set("FEED".to_owned()),
        aggregate_id: Set(claim.feed_id.clone()),
        refresh_id: Set(claim.run_id.clone()),
        event_sequence: Set(20),
        payload_version: Set(1),
        payload_json: Set("{\"conflict\":true}".to_owned()),
        idempotency_key: Set(format!("refresh:{}:completed:v1", claim.run_id)),
        status: Set("PENDING".to_owned()),
        available_at: Set(now),
        attempts: Set(0),
        lease_owner: Set(None),
        lease_until: Set(None),
        created_at: Set(now),
        completed_at: Set(None),
    }
    .insert(database)
    .await
    .unwrap();
}
