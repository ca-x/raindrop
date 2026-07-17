#[allow(dead_code)]
mod support;

use std::time::Duration;

use raindrop::{
    db::{
        entities::{feed, feed_refresh_run},
        migrate, rollback,
    },
    feeds::{
        ClaimRequest, FeedRepository, QueueRefreshRequest, RefreshCounts, RefreshFailure,
        RefreshRepositoryError, RefreshStatus, RefreshTrigger,
    },
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseBackend, EntityTrait, Statement,
    TransactionTrait,
};
use secrecy::SecretString;
use support::database::{FEED_ID, connect_for_contract, insert_feed};
use tempfile::TempDir;
use time::{OffsetDateTime, macros::datetime};

const RUN_ID: &str = "00000000-0000-4000-8000-000000000401";
const SECOND_RUN_ID: &str = "00000000-0000-4000-8000-000000000402";
const RETRY_AT: OffsetDateTime = datetime!(2026-07-18 12:34:56.123456 UTC);

#[tokio::test]
async fn sqlite_refresh_claim_contract() {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("shared-backend-contract.db").display()
    );
    backend_refresh_claim_contract(url).await;
}

#[tokio::test]
async fn postgres_refresh_claim_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("postgres refresh claim contract skipped: test database URL is not configured");
        return;
    };
    backend_refresh_claim_contract(url).await;
}

#[tokio::test]
async fn mysql_refresh_claim_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("mysql refresh claim contract skipped: test database URL is not configured");
        return;
    };
    backend_refresh_claim_contract(url).await;
}

#[tokio::test]
async fn sqlite_one_claimant_wins_a_queued_refresh() {
    let (_data, _url, database) = sqlite_database("one-claimant").await;
    seed_claimable_run(&database).await;
    let repository = FeedRepository::new(database.clone());

    let claim = repository
        .claim_due(ClaimRequest {
            owner: "worker-a".to_owned(),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("claim should not fail")
        .expect("queued refresh should be claimable");

    assert_eq!(claim.run_id, RUN_ID);
    assert_eq!(claim.feed_id, FEED_ID);
    assert_eq!(claim.owner, "worker-a");
    assert_eq!(claim.lease_token, 2);

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_two_concurrent_claimants_have_exactly_one_winner() {
    let (_data, url, first_database) = sqlite_database("two-claimants").await;
    seed_claimable_run(&first_database).await;
    let second_database = connect_for_contract(SecretString::from(url)).await;
    let first_repository = FeedRepository::new(first_database.clone());
    let second_repository = FeedRepository::new(second_database.clone());

    let first = first_repository.claim_due(ClaimRequest {
        owner: "worker-a".to_owned(),
        lease_duration: Duration::from_secs(30),
    });
    let second = second_repository.claim_due(ClaimRequest {
        owner: "worker-b".to_owned(),
        lease_duration: Duration::from_secs(30),
    });
    let (first, second) = tokio::join!(first, second);
    let outcomes = [
        first.expect("first claim should not fail"),
        second.expect("second claim should not fail"),
    ];

    assert_eq!(outcomes.iter().filter(|claim| claim.is_some()).count(), 1);
    assert_eq!(outcomes.iter().filter(|claim| claim.is_none()).count(), 1);

    first_database
        .close()
        .await
        .expect("first database should close");
    second_database
        .close()
        .await
        .expect("second database should close");
}

#[tokio::test]
async fn sqlite_newer_token_fences_an_old_worker() {
    let (_data, _url, database) = sqlite_database("newer-token-fences-old").await;
    seed_claimable_run(&database).await;
    let repository = FeedRepository::new(database.clone());
    let first = repository
        .claim_due(ClaimRequest {
            owner: "worker-a".to_owned(),
            lease_duration: Duration::from_millis(250),
        })
        .await
        .expect("first claim should not fail")
        .expect("first run should be claimable");

    let extended = repository
        .extend_lease(&first, Duration::from_millis(40))
        .await
        .expect("current owner should extend its live lease");
    assert_eq!(extended.lease_token, first.lease_token);
    insert_queued_run(&database, SECOND_RUN_ID, "scheduled:two").await;
    tokio::time::sleep(Duration::from_millis(80)).await;

    let second = repository
        .claim_due(ClaimRequest {
            owner: "worker-b".to_owned(),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("second claim should not fail")
        .expect("second run should become claimable after expiry");
    assert_eq!(second.run_id, SECOND_RUN_ID);
    assert_eq!(second.lease_token, first.lease_token + 1);

    let old_extend = repository
        .extend_lease(&first, Duration::from_secs(30))
        .await;
    assert!(matches!(old_extend, Err(RefreshRepositoryError::LeaseLost)));

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_lock_wait_crossing_the_deadline_cannot_complete() {
    let (_data, url, database) = sqlite_database("lock-wait-crosses-deadline").await;
    seed_claimable_run(&database).await;
    let repository = FeedRepository::new(database.clone());
    let claim = repository
        .claim_due(ClaimRequest {
            owner: "worker-a".to_owned(),
            lease_duration: Duration::from_millis(150),
        })
        .await
        .expect("claim should not fail")
        .expect("run should be claimable");

    let blocker = connect_for_contract(SecretString::from(url)).await;
    blocker
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "BEGIN IMMEDIATE".to_owned(),
        ))
        .await
        .expect("blocking write transaction should start");

    let completion = tokio::spawn(async move {
        repository
            .complete_success(
                &claim,
                200,
                RefreshCounts {
                    new_count: 0,
                    updated_count: 0,
                    dropped_count: 0,
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    blocker
        .execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            "COMMIT".to_owned(),
        ))
        .await
        .expect("blocking write transaction should commit");

    let result = completion.await.expect("completion task should join");
    assert!(matches!(result, Err(RefreshRepositoryError::LeaseLost)));

    database.close().await.expect("database should close");
    blocker.close().await.expect("blocker should close");
}

#[tokio::test]
async fn sqlite_manual_idempotency_returns_existing_or_conflicts() {
    let (_data, _url, database) = sqlite_database("manual-idempotency").await;
    seed_feed(&database).await;
    let repository = FeedRepository::new(database.clone());
    let request = QueueRefreshRequest {
        feed_id: FEED_ID.to_owned(),
        requested_by_user_id: None,
        trigger: RefreshTrigger::Manual,
        idempotency_key: "manual:stable-key".to_owned(),
    };

    let first = repository
        .queue_refresh(request.clone())
        .await
        .expect("manual refresh should queue");
    let retry = repository
        .queue_refresh(request.clone())
        .await
        .expect("same manual retry should return existing run");
    assert_eq!(retry, first);

    let trigger_conflict = repository
        .queue_refresh(QueueRefreshRequest {
            trigger: RefreshTrigger::Retry,
            ..request.clone()
        })
        .await;
    assert!(matches!(
        trigger_conflict,
        Err(RefreshRepositoryError::IdempotencyConflict)
    ));

    let requester_conflict = repository
        .queue_refresh(QueueRefreshRequest {
            requested_by_user_id: Some("00000000-0000-4000-8000-000000000099".to_owned()),
            ..request
        })
        .await;
    assert!(matches!(
        requester_conflict,
        Err(RefreshRepositoryError::IdempotencyConflict)
    ));

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_terminal_transitions_require_running_prior_state() {
    let (_data, _url, database) = sqlite_database("terminal-state-machine").await;
    seed_claimable_run(&database).await;
    let repository = FeedRepository::new(database.clone());

    let success_claim = claim_for(&repository, "worker-success").await;
    repository
        .complete_success(
            &success_claim,
            200,
            RefreshCounts {
                new_count: 3,
                updated_count: 2,
                dropped_count: 1,
            },
        )
        .await
        .expect("running refresh should complete successfully");
    assert_terminal_run(
        &database,
        &success_claim,
        RefreshStatus::Success,
        Some(200),
        RefreshCounts {
            new_count: 3,
            updated_count: 2,
            dropped_count: 1,
        },
        None,
        None,
    )
    .await;
    assert!(matches!(
        repository.complete_not_modified(&success_claim).await,
        Err(RefreshRepositoryError::InvalidTransition)
    ));

    insert_queued_run(&database, SECOND_RUN_ID, "scheduled:two").await;
    let not_modified_claim = claim_for(&repository, "worker-not-modified").await;
    repository
        .complete_not_modified(&not_modified_claim)
        .await
        .expect("running refresh should complete as not modified");
    assert_terminal_run(
        &database,
        &not_modified_claim,
        RefreshStatus::NotModified,
        Some(304),
        RefreshCounts {
            new_count: 0,
            updated_count: 0,
            dropped_count: 0,
        },
        None,
        None,
    )
    .await;
    assert!(matches!(
        repository
            .complete_failure(
                &not_modified_claim,
                RefreshFailure {
                    error_code: "FETCH_FAILED".to_owned(),
                    http_status: Some(503),
                    retry_at: Some(RETRY_AT),
                },
            )
            .await,
        Err(RefreshRepositoryError::InvalidTransition)
    ));

    let third_run_id = "00000000-0000-4000-8000-000000000403";
    insert_queued_run(&database, third_run_id, "scheduled:three").await;
    let failure_claim = claim_for(&repository, "worker-error").await;
    repository
        .complete_failure(
            &failure_claim,
            RefreshFailure {
                error_code: "FETCH_FAILED".to_owned(),
                http_status: Some(503),
                retry_at: Some(RETRY_AT),
            },
        )
        .await
        .expect("running refresh should complete as error");
    assert_terminal_run(
        &database,
        &failure_claim,
        RefreshStatus::Error,
        Some(503),
        RefreshCounts {
            new_count: 0,
            updated_count: 0,
            dropped_count: 0,
        },
        Some("FETCH_FAILED"),
        Some(RETRY_AT),
    )
    .await;
    assert!(matches!(
        repository
            .complete_success(
                &failure_claim,
                200,
                RefreshCounts {
                    new_count: 0,
                    updated_count: 0,
                    dropped_count: 0,
                },
            )
            .await,
        Err(RefreshRepositoryError::InvalidTransition)
    ));

    let partial_run_id = "00000000-0000-4000-8000-000000000404";
    insert_queued_run(&database, partial_run_id, "scheduled:partial").await;
    let partial_claim = claim_for(&repository, "worker-partial").await;
    repository
        .complete_partial(
            &partial_claim,
            200,
            RefreshCounts {
                new_count: 2,
                updated_count: 1,
                dropped_count: 3,
            },
        )
        .await
        .expect("running refresh should complete as partial");
    assert_terminal_run(
        &database,
        &partial_claim,
        RefreshStatus::Partial,
        Some(200),
        RefreshCounts {
            new_count: 2,
            updated_count: 1,
            dropped_count: 3,
        },
        None,
        None,
    )
    .await;
    assert!(matches!(
        repository.complete_not_modified(&partial_claim).await,
        Err(RefreshRepositoryError::InvalidTransition)
    ));

    let cancelled_run_id = "00000000-0000-4000-8000-000000000405";
    insert_queued_run(&database, cancelled_run_id, "scheduled:cancel-running").await;
    let cancelled_claim = claim_for(&repository, "worker-cancel-running").await;
    repository
        .cancel_running(&cancelled_claim)
        .await
        .expect("owned running refresh should cancel explicitly");
    assert_terminal_run(
        &database,
        &cancelled_claim,
        RefreshStatus::Cancelled,
        None,
        RefreshCounts {
            new_count: 0,
            updated_count: 0,
            dropped_count: 0,
        },
        None,
        None,
    )
    .await;
    assert!(matches!(
        repository.complete_not_modified(&cancelled_claim).await,
        Err(RefreshRepositoryError::InvalidTransition)
    ));

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_scheduled_claims_obey_database_due_time_but_manual_runs_do_not() {
    let (_data, _url, database) = sqlite_database("database-due-time").await;
    seed_feed(&database).await;
    insert_queued_run(&database, RUN_ID, "scheduled:future").await;
    let repository = FeedRepository::new(database.clone());

    let scheduled = repository
        .claim_due(ClaimRequest {
            owner: "worker-scheduled".to_owned(),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("scheduled claim should not fail");
    assert!(scheduled.is_none());

    feed_refresh_run::Entity::delete_by_id(RUN_ID)
        .exec(&database)
        .await
        .expect("future scheduled fixture should delete");
    insert_queued_run_with_trigger(
        &database,
        RUN_ID,
        "manual:immediate",
        RefreshTrigger::Manual,
    )
    .await;
    let manual = repository
        .claim_due(ClaimRequest {
            owner: "worker-manual".to_owned(),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("manual claim should not fail");
    assert!(manual.is_some());

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_shifted_application_deadlines_do_not_authorize_writes() {
    let (_data, _url, database) = sqlite_database("application-clock-shifts").await;
    seed_claimable_run(&database).await;
    let repository = FeedRepository::new(database.clone());
    let claim = claim_for(&repository, "worker-clock-skew").await;

    let mut backward_shifted = claim.clone();
    backward_shifted.lease_deadline = OffsetDateTime::UNIX_EPOCH;
    let extended = repository
        .extend_lease(&backward_shifted, Duration::from_millis(40))
        .await
        .expect("diagnostic past deadline must not reject a live database lease");

    let mut forward_shifted = extended;
    forward_shifted.lease_deadline = OffsetDateTime::UNIX_EPOCH + time::Duration::days(100_000);
    tokio::time::sleep(Duration::from_millis(80)).await;
    let completion = repository.complete_not_modified(&forward_shifted).await;
    assert!(matches!(completion, Err(RefreshRepositoryError::LeaseLost)));

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_lease_token_boundaries_are_typed_and_monotonic() {
    let (_data, _url, database) = sqlite_database("lease-token-boundaries").await;
    seed_claimable_run(&database).await;
    let repository = FeedRepository::new(database.clone());

    set_feed_token(&database, i64::MAX).await;
    assert!(matches!(
        repository
            .claim_due(ClaimRequest {
                owner: "worker-exhausted".to_owned(),
                lease_duration: Duration::from_secs(30),
            })
            .await,
        Err(RefreshRepositoryError::TokenExhausted)
    ));

    set_feed_token(&database, -1).await;
    assert!(matches!(
        repository
            .claim_due(ClaimRequest {
                owner: "worker-corrupt".to_owned(),
                lease_duration: Duration::from_secs(30),
            })
            .await,
        Err(RefreshRepositoryError::CorruptData)
    ));

    set_feed_token(&database, i64::MAX - 1).await;
    let last = repository
        .claim_due(ClaimRequest {
            owner: "worker-last-token".to_owned(),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("last representable token should claim")
        .expect("queued run should be claimable");
    assert_eq!(last.lease_token, i64::MAX);

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_system_transitions_are_explicit_and_state_checked() {
    let (_data, _url, database) = sqlite_database("system-transitions").await;
    seed_claimable_run(&database).await;
    let repository = FeedRepository::new(database.clone());

    repository
        .cancel_queued(RUN_ID)
        .await
        .expect("queued run should cancel");
    assert_queued_cancellation(&database, RUN_ID).await;
    assert!(matches!(
        repository.cancel_queued(RUN_ID).await,
        Err(RefreshRepositoryError::InvalidTransition)
    ));
    assert!(
        repository
            .claim_due(ClaimRequest {
                owner: "worker-cancelled".to_owned(),
                lease_duration: Duration::from_secs(30),
            })
            .await
            .expect("claim after cancellation should not fail")
            .is_none()
    );

    insert_queued_run(&database, SECOND_RUN_ID, "scheduled:lease-lost").await;
    let claim = repository
        .claim_due(ClaimRequest {
            owner: "worker-lease-lost".to_owned(),
            lease_duration: Duration::from_millis(40),
        })
        .await
        .expect("claim should not fail")
        .expect("queued run should claim");
    assert!(matches!(
        repository.record_lease_lost(&claim.run_id).await,
        Err(RefreshRepositoryError::InvalidTransition)
    ));
    tokio::time::sleep(Duration::from_millis(80)).await;
    repository
        .record_lease_lost(&claim.run_id)
        .await
        .expect("expired running run should record lease loss");
    assert_terminal_run(
        &database,
        &claim,
        RefreshStatus::LeaseLost,
        None,
        RefreshCounts {
            new_count: 0,
            updated_count: 0,
            dropped_count: 0,
        },
        None,
        None,
    )
    .await;
    assert!(matches!(
        repository.record_lease_lost(&claim.run_id).await,
        Err(RefreshRepositoryError::InvalidTransition)
    ));

    database.close().await.expect("database should close");
}

async fn backend_refresh_claim_contract(url: String) {
    concurrent_claim_contract(&url).await;
    newer_token_fencing_contract(&url).await;
    lock_wait_deadline_contract(&url).await;
    diagnostic_deadline_contract(&url).await;
    manual_idempotency_contract(&url).await;
    terminal_transition_contract(&url).await;

    let database = reset_contract_database(&url).await;
    if database.get_database_backend() == DatabaseBackend::MySql {
        database.close().await.expect("database should close");
        mysql_lock_order_contract(&url).await;
    } else {
        database.close().await.expect("database should close");
    }
}

async fn reset_contract_database(url: &str) -> sea_orm::DatabaseConnection {
    let database = connect_for_contract(SecretString::from(url.to_owned())).await;
    rollback(&database)
        .await
        .unwrap_or_else(|_| panic!("dedicated refresh contract database should reset"));
    migrate(&database)
        .await
        .expect("refresh claim migrations should apply");
    database
}

async fn concurrent_claim_contract(url: &str) {
    let first_database = reset_contract_database(url).await;
    seed_claimable_run(&first_database).await;
    let second_database = connect_for_contract(SecretString::from(url.to_owned())).await;
    let first_repository = FeedRepository::new(first_database.clone());
    let second_repository = FeedRepository::new(second_database.clone());

    let first = first_repository.claim_due(ClaimRequest {
        owner: "worker-contract-a".to_owned(),
        lease_duration: Duration::from_secs(30),
    });
    let second = second_repository.claim_due(ClaimRequest {
        owner: "worker-contract-b".to_owned(),
        lease_duration: Duration::from_secs(30),
    });
    let (first, second) = tokio::join!(first, second);
    let outcomes = [
        first.expect("first backend claim should not fail"),
        second.expect("second backend claim should not fail"),
    ];
    assert_eq!(outcomes.iter().filter(|claim| claim.is_some()).count(), 1);
    assert_eq!(outcomes.iter().filter(|claim| claim.is_none()).count(), 1);

    first_database
        .close()
        .await
        .expect("first database should close");
    second_database
        .close()
        .await
        .expect("second database should close");
}

async fn newer_token_fencing_contract(url: &str) {
    let database = reset_contract_database(url).await;
    seed_claimable_run(&database).await;
    let repository = FeedRepository::new(database.clone());
    let old_claim = repository
        .claim_due(ClaimRequest {
            owner: "worker-contract-old".to_owned(),
            lease_duration: Duration::from_millis(250),
        })
        .await
        .expect("old backend claim should not fail")
        .expect("old backend run should claim");
    repository
        .extend_lease(&old_claim, Duration::from_millis(40))
        .await
        .expect("old backend lease should extend");
    insert_queued_run(&database, SECOND_RUN_ID, "contract:new-token").await;
    tokio::time::sleep(Duration::from_millis(80)).await;

    let new_claim = repository
        .claim_due(ClaimRequest {
            owner: "worker-contract-new".to_owned(),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("new backend claim should not fail")
        .expect("new backend run should claim");
    assert_eq!(new_claim.lease_token, old_claim.lease_token + 1);
    assert!(matches!(
        repository
            .extend_lease(&old_claim, Duration::from_secs(30))
            .await,
        Err(RefreshRepositoryError::LeaseLost)
    ));

    database.close().await.expect("database should close");
}

async fn lock_wait_deadline_contract(url: &str) {
    let database = reset_contract_database(url).await;
    seed_claimable_run(&database).await;
    let backend = database.get_database_backend();
    let repository = FeedRepository::new(database.clone());
    let claim = repository
        .claim_due(ClaimRequest {
            owner: "worker-contract-lock-wait".to_owned(),
            lease_duration: Duration::from_millis(150),
        })
        .await
        .expect("backend claim should not fail")
        .expect("backend run should claim");
    let blocker = connect_for_contract(SecretString::from(url.to_owned())).await;

    let result = if backend == DatabaseBackend::Sqlite {
        blocker
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "BEGIN IMMEDIATE".to_owned(),
            ))
            .await
            .expect("SQLite blocking transaction should start");
        let completion = tokio::spawn(complete_empty_success(repository, claim));
        tokio::time::sleep(Duration::from_millis(250)).await;
        blocker
            .execute(Statement::from_string(
                DatabaseBackend::Sqlite,
                "COMMIT".to_owned(),
            ))
            .await
            .expect("SQLite blocking transaction should commit");
        completion.await.expect("completion task should join")
    } else {
        let transaction = blocker
            .begin()
            .await
            .expect("blocking transaction should start");
        let sql = if backend == DatabaseBackend::Postgres {
            "SELECT id FROM feeds WHERE id = $1 FOR UPDATE"
        } else {
            "SELECT id FROM feeds WHERE id = ? FOR UPDATE"
        };
        transaction
            .query_one(Statement::from_sql_and_values(
                backend,
                sql,
                [FEED_ID.into()],
            ))
            .await
            .expect("feed row should lock")
            .expect("feed row should exist");
        let completion = tokio::spawn(complete_empty_success(repository, claim));
        tokio::time::sleep(Duration::from_millis(250)).await;
        transaction
            .commit()
            .await
            .expect("blocking transaction should commit");
        completion.await.expect("completion task should join")
    };
    assert!(matches!(result, Err(RefreshRepositoryError::LeaseLost)));

    database.close().await.expect("database should close");
    blocker.close().await.expect("blocker should close");
}

async fn complete_empty_success(
    repository: FeedRepository,
    claim: raindrop::feeds::RefreshClaim,
) -> Result<(), RefreshRepositoryError> {
    repository
        .complete_success(
            &claim,
            200,
            RefreshCounts {
                new_count: 0,
                updated_count: 0,
                dropped_count: 0,
            },
        )
        .await
}

async fn diagnostic_deadline_contract(url: &str) {
    let database = reset_contract_database(url).await;
    seed_claimable_run(&database).await;
    let repository = FeedRepository::new(database.clone());
    let mut claim = claim_for(&repository, "worker-contract-clock").await;
    claim.lease_deadline = OffsetDateTime::UNIX_EPOCH;
    let mut claim = repository
        .extend_lease(&claim, Duration::from_millis(40))
        .await
        .expect("past diagnostic deadline should not reject a live backend lease");
    claim.lease_deadline = OffsetDateTime::UNIX_EPOCH + time::Duration::days(100_000);
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(matches!(
        repository.complete_not_modified(&claim).await,
        Err(RefreshRepositoryError::LeaseLost)
    ));

    database.close().await.expect("database should close");
}

async fn manual_idempotency_contract(url: &str) {
    let database = reset_contract_database(url).await;
    seed_feed(&database).await;
    let repository = FeedRepository::new(database.clone());
    let request = QueueRefreshRequest {
        feed_id: FEED_ID.to_owned(),
        requested_by_user_id: None,
        trigger: RefreshTrigger::Manual,
        idempotency_key: "manual:shared-backend-contract".to_owned(),
    };
    let first = repository
        .queue_refresh(request.clone())
        .await
        .expect("backend refresh should queue");
    assert_eq!(
        repository
            .queue_refresh(request.clone())
            .await
            .expect("backend idempotent retry should return existing"),
        first
    );
    assert!(matches!(
        repository
            .queue_refresh(QueueRefreshRequest {
                trigger: RefreshTrigger::Retry,
                ..request.clone()
            })
            .await,
        Err(RefreshRepositoryError::IdempotencyConflict)
    ));
    assert!(matches!(
        repository
            .queue_refresh(QueueRefreshRequest {
                requested_by_user_id: Some("00000000-0000-4000-8000-000000000099".to_owned()),
                ..request
            })
            .await,
        Err(RefreshRepositoryError::IdempotencyConflict)
    ));

    database.close().await.expect("database should close");
}

async fn terminal_transition_contract(url: &str) {
    let database = reset_contract_database(url).await;
    seed_claimable_run(&database).await;
    let repository = FeedRepository::new(database.clone());

    let success_claim = claim_for(&repository, "worker-contract-success").await;
    let success_counts = RefreshCounts {
        new_count: 3,
        updated_count: 2,
        dropped_count: 1,
    };
    repository
        .complete_success(&success_claim, 200, success_counts)
        .await
        .expect("backend success should complete");
    assert_terminal_run(
        &database,
        &success_claim,
        RefreshStatus::Success,
        Some(200),
        success_counts,
        None,
        None,
    )
    .await;
    assert!(matches!(
        repository.complete_not_modified(&success_claim).await,
        Err(RefreshRepositoryError::InvalidTransition)
    ));

    insert_queued_run(&database, SECOND_RUN_ID, "contract:not-modified").await;
    let not_modified_claim = claim_for(&repository, "worker-contract-not-modified").await;
    repository
        .complete_not_modified(&not_modified_claim)
        .await
        .expect("backend 304 should complete");
    assert_terminal_run(
        &database,
        &not_modified_claim,
        RefreshStatus::NotModified,
        Some(304),
        RefreshCounts {
            new_count: 0,
            updated_count: 0,
            dropped_count: 0,
        },
        None,
        None,
    )
    .await;

    let error_run_id = "00000000-0000-4000-8000-000000000403";
    insert_queued_run(&database, error_run_id, "contract:error").await;
    let error_claim = claim_for(&repository, "worker-contract-error").await;
    repository
        .complete_failure(
            &error_claim,
            RefreshFailure {
                error_code: "FETCH_FAILED".to_owned(),
                http_status: Some(503),
                retry_at: Some(RETRY_AT),
            },
        )
        .await
        .expect("backend error should complete");
    assert_terminal_run(
        &database,
        &error_claim,
        RefreshStatus::Error,
        Some(503),
        RefreshCounts {
            new_count: 0,
            updated_count: 0,
            dropped_count: 0,
        },
        Some("FETCH_FAILED"),
        Some(RETRY_AT),
    )
    .await;
    assert!(matches!(
        repository.complete_not_modified(&error_claim).await,
        Err(RefreshRepositoryError::InvalidTransition)
    ));

    database.close().await.expect("database should close");
}

async fn mysql_lock_order_contract(url: &str) {
    let database = reset_contract_database(url).await;
    seed_claimable_run(&database).await;
    let completion_repository = FeedRepository::new(database.clone());
    let claim = completion_repository
        .claim_due(ClaimRequest {
            owner: "worker-mysql-lock-order".to_owned(),
            lease_duration: Duration::from_secs(5),
        })
        .await
        .expect("MySQL lock-order claim should not fail")
        .expect("MySQL lock-order run should claim");
    let recorder_database = connect_for_contract(SecretString::from(url.to_owned())).await;
    let recorder = FeedRepository::new(recorder_database.clone());
    let blocker = connect_for_contract(SecretString::from(url.to_owned())).await;
    let transaction = blocker
        .begin()
        .await
        .expect("MySQL run blocker should start");
    transaction
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::MySql,
            "SELECT id FROM feed_refresh_runs WHERE id = ? FOR UPDATE",
            [claim.run_id.as_str().into()],
        ))
        .await
        .expect("MySQL run row should lock")
        .expect("MySQL run row should exist");

    let run_id = claim.run_id.clone();
    let lease_loss = tokio::spawn(async move { recorder.record_lease_lost(&run_id).await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    let completion = tokio::spawn(complete_empty_success(completion_repository, claim));
    tokio::time::sleep(Duration::from_millis(30)).await;
    transaction
        .commit()
        .await
        .expect("MySQL run blocker should commit");

    let lease_loss = tokio::time::timeout(Duration::from_secs(5), lease_loss)
        .await
        .expect("MySQL lease-loss transition must not deadlock")
        .expect("MySQL lease-loss task should join");
    let completion = tokio::time::timeout(Duration::from_secs(5), completion)
        .await
        .expect("MySQL owned completion must not deadlock")
        .expect("MySQL completion task should join");
    assert!(matches!(
        lease_loss,
        Err(RefreshRepositoryError::InvalidTransition)
    ));
    completion.expect("MySQL owned completion should succeed after lock serialization");

    database.close().await.expect("database should close");
    recorder_database
        .close()
        .await
        .expect("recorder database should close");
    blocker.close().await.expect("blocker should close");
}

async fn set_feed_token(database: &sea_orm::DatabaseConnection, token: i64) {
    let feed = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = feed.into();
    active.lease_token = Set(token);
    active
        .update(database)
        .await
        .expect("feed token fixture should update");
}

async fn assert_terminal_run(
    database: &sea_orm::DatabaseConnection,
    claim: &raindrop::feeds::RefreshClaim,
    expected_status: RefreshStatus,
    expected_http_status: Option<i32>,
    expected_counts: RefreshCounts,
    expected_error_code: Option<&str>,
    expected_retry_at: Option<OffsetDateTime>,
) {
    let run = feed_refresh_run::Entity::find_by_id(&claim.run_id)
        .one(database)
        .await
        .expect("terminal refresh should query")
        .expect("terminal refresh should exist");
    assert_eq!(run.status, expected_status.as_str());
    assert_eq!(run.http_status, expected_http_status);
    assert_eq!(run.new_count, expected_counts.new_count);
    assert_eq!(run.updated_count, expected_counts.updated_count);
    assert_eq!(run.dropped_count, expected_counts.dropped_count);
    assert_eq!(run.error_code.as_deref(), expected_error_code);
    assert_eq!(run.retry_at, expected_retry_at);
    assert_eq!(run.lease_token, Some(claim.lease_token));
    assert!(run.started_at.is_some());
    assert!(run.completed_at.is_some());
    assert!(run.completed_at >= run.started_at);
    assert_eq!(run.commit_generation, None);
    assert_eq!(run.fetched_at, None);
    assert_eq!(run.persisted_at, None);
}

async fn assert_queued_cancellation(database: &sea_orm::DatabaseConnection, run_id: &str) {
    let run = feed_refresh_run::Entity::find_by_id(run_id)
        .one(database)
        .await
        .expect("cancelled queued refresh should query")
        .expect("cancelled queued refresh should exist");
    assert_eq!(run.status, RefreshStatus::Cancelled.as_str());
    assert_eq!(run.http_status, None);
    assert_eq!(
        (run.new_count, run.updated_count, run.dropped_count),
        (0, 0, 0)
    );
    assert_eq!(run.error_code, None);
    assert_eq!(run.retry_at, None);
    assert_eq!(run.lease_token, None);
    assert_eq!(run.started_at, None);
    assert!(run.completed_at.is_some());
    assert_eq!(run.commit_generation, None);
    assert_eq!(run.fetched_at, None);
    assert_eq!(run.persisted_at, None);
}

async fn claim_for(repository: &FeedRepository, owner: &str) -> raindrop::feeds::RefreshClaim {
    repository
        .claim_due(ClaimRequest {
            owner: owner.to_owned(),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("claim should not fail")
        .expect("queued run should be claimable")
}

async fn sqlite_database(name: &str) -> (TempDir, String, sea_orm::DatabaseConnection) {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join(format!("{name}.db")).display()
    );
    let database = connect_for_contract(SecretString::from(url.clone())).await;
    migrate(&database)
        .await
        .expect("refresh claim migrations should apply");
    (data, url, database)
}

async fn seed_claimable_run(database: &sea_orm::DatabaseConnection) {
    seed_feed(database).await;
    let feed = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = feed.into();
    active.next_fetch_at = Set(OffsetDateTime::now_utc() - time::Duration::minutes(1));
    active
        .update(database)
        .await
        .expect("feed should become due");
    insert_queued_run(database, RUN_ID, "scheduled:one").await;
}

async fn seed_feed(database: &sea_orm::DatabaseConnection) {
    let now = OffsetDateTime::now_utc();
    insert_feed(database, now).await;
    let feed = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = feed.into();
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.update(database).await.expect("feed should unlock");
}

async fn insert_queued_run(
    database: &sea_orm::DatabaseConnection,
    run_id: &str,
    idempotency_key: &str,
) {
    insert_queued_run_with_trigger(database, run_id, idempotency_key, RefreshTrigger::Scheduled)
        .await;
}

async fn insert_queued_run_with_trigger(
    database: &sea_orm::DatabaseConnection,
    run_id: &str,
    idempotency_key: &str,
    trigger: RefreshTrigger,
) {
    feed_refresh_run::ActiveModel {
        id: Set(run_id.to_owned()),
        feed_id: Set(FEED_ID.to_owned()),
        requested_by_user_id: Set(None),
        trigger_kind: Set(trigger.as_str().to_owned()),
        status: Set("QUEUED".to_owned()),
        idempotency_key: Set(idempotency_key.to_owned()),
        lease_token: Set(None),
        commit_generation: Set(None),
        queued_at: Set(OffsetDateTime::now_utc()),
        started_at: Set(None),
        fetched_at: Set(None),
        persisted_at: Set(None),
        completed_at: Set(None),
        http_status: Set(None),
        new_count: Set(0),
        updated_count: Set(0),
        dropped_count: Set(0),
        error_code: Set(None),
        retry_at: Set(None),
    }
    .insert(database)
    .await
    .expect("queued run should insert");
}
