#[allow(dead_code)]
mod support;

use raindrop::db::{
    entities::{content_artifact, content_job, content_job_attempt, content_job_result},
    migrate, rollback,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseBackend, EntityTrait, QueryResult,
    Statement, TryGetable,
};
use secrecy::SecretString;
use support::database::{
    ENTRY_A_ID, HASH_A, HASH_B, HASH_C, USER_A_ID, connect_for_contract, insert_entry, insert_feed,
    insert_user,
};
use time::macros::datetime;

const JOB_ID: &str = "00000000-0000-4000-8000-000000001001";
const ATTEMPT_ID: &str = "00000000-0000-4000-8000-000000001002";
const ARTIFACT_ID: &str = "00000000-0000-4000-8000-000000001003";

const EXPECTED_INDEXES: &[&str] = &[
    "uq_content_jobs_idempotency",
    "idx_content_jobs_due",
    "idx_content_jobs_user_status",
    "idx_content_jobs_entry",
    "idx_content_jobs_identity",
    "uq_content_job_attempt_number",
    "idx_content_attempts_job",
    "uq_content_artifact_identity",
    "idx_content_artifacts_entry",
    "idx_content_artifacts_producer",
    "idx_content_job_results_artifact",
];

#[tokio::test]
async fn sqlite_content_job_migration_and_entities_round_trip() {
    let data = tempfile::tempdir().expect("temporary directory should create");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("content-job-migration.db").display()
    );
    let database = connect_for_contract(SecretString::from(url)).await;
    migrate(&database).await.expect("migrations should apply");

    assert_tables(&database, true).await;
    assert_indexes(&database).await;

    let now = datetime!(2026-07-19 12:00:00 UTC);
    insert_user(&database, USER_A_ID, "content-user").await;
    insert_feed(&database, now).await;
    insert_entry(
        &database,
        ENTRY_A_ID,
        1,
        "content-entry",
        HASH_A,
        Some(1_752_926_400_000_000),
        now,
    )
    .await;

    content_job::ActiveModel {
        id: Set(JOB_ID.to_owned()),
        user_id: Set(USER_A_ID.to_owned()),
        entry_id: Set(ENTRY_A_ID.to_owned()),
        operation: Set("SUMMARIZE".to_owned()),
        artifact_kind: Set("AI_SUMMARY".to_owned()),
        target_locale: Set(None),
        trigger_kind: Set("MANUAL_API".to_owned()),
        plugin_key: Set("raindrop.ai-content".to_owned()),
        plugin_version: Set("1.0.0".to_owned()),
        component_digest: Set(HASH_A.to_owned()),
        provider_binding_id: Set("00000000-0000-4000-8000-000000000901".to_owned()),
        provider_kind: Set("OPENAI_RESPONSES".to_owned()),
        provider_model: Set("gpt-5-mini".to_owned()),
        provider_revision: Set(0),
        prompt_version: Set("summary-v1".to_owned()),
        schema_id: Set("raindrop://schemas/artifacts/ai-summary/v1".to_owned()),
        entry_content_hash: Set(HASH_A.to_owned()),
        input_hash: Set(HASH_B.to_owned()),
        config_hash: Set(HASH_C.to_owned()),
        mcp_provenance_hash: Set(HASH_A.to_owned()),
        artifact_identity_hash: Set(HASH_B.to_owned()),
        idempotency_key: Set("manual-content-job".to_owned()),
        idempotency_key_hash: Set(HASH_C.to_owned()),
        request_hash: Set(HASH_A.to_owned()),
        call_chain_id: Set("chain-content-job".to_owned()),
        remaining_depth: Set(4),
        status: Set("RUNNING".to_owned()),
        attempts: Set(1),
        max_attempts: Set(3),
        timeout_seconds: Set(180),
        next_attempt_at: Set(now),
        lease_owner: Set(Some("worker-content".to_owned())),
        lease_token: Set(1),
        lease_until: Set(Some(now + time::Duration::seconds(30))),
        attempt_deadline_at: Set(Some(now + time::Duration::seconds(180))),
        last_error_code: Set(None),
        created_at: Set(now),
        started_at: Set(Some(now)),
        completed_at: Set(None),
    }
    .insert(&database)
    .await
    .expect("content job should insert");

    content_job_attempt::ActiveModel {
        id: Set(ATTEMPT_ID.to_owned()),
        job_id: Set(JOB_ID.to_owned()),
        attempt: Set(1),
        lease_token: Set(1),
        status: Set("SUCCEEDED".to_owned()),
        started_at: Set(now),
        deadline_at: Set(now + time::Duration::seconds(180)),
        completed_at: Set(Some(now + time::Duration::seconds(2))),
        error_code: Set(None),
        retryable: Set(Some(false)),
        outcome_unknown: Set(false),
        provider_request_count: Set(1),
        mcp_call_count: Set(0),
        input_tokens: Set(100),
        output_tokens: Set(20),
        estimated_cost_micros: Set(42),
        execution_metadata_json: Set("{\"schemaVersion\":1}".to_owned()),
    }
    .insert(&database)
    .await
    .expect("content job attempt should insert");

    content_artifact::ActiveModel {
        id: Set(ARTIFACT_ID.to_owned()),
        user_id: Set(USER_A_ID.to_owned()),
        entry_id: Set(ENTRY_A_ID.to_owned()),
        producer_job_id: Set(JOB_ID.to_owned()),
        kind: Set("AI_SUMMARY".to_owned()),
        locale: Set(None),
        schema_id: Set("raindrop://schemas/artifacts/ai-summary/v1".to_owned()),
        entry_content_hash: Set(HASH_A.to_owned()),
        input_hash: Set(HASH_B.to_owned()),
        config_hash: Set(HASH_C.to_owned()),
        processor_key: Set("raindrop.ai-content".to_owned()),
        processor_version: Set("1.0.0".to_owned()),
        component_digest: Set(HASH_A.to_owned()),
        provider_binding_id: Set("00000000-0000-4000-8000-000000000901".to_owned()),
        provider_kind: Set("OPENAI_RESPONSES".to_owned()),
        provider_model: Set("gpt-5-mini".to_owned()),
        provider_revision: Set(0),
        provider_label: Set("OpenAI".to_owned()),
        prompt_version: Set("summary-v1".to_owned()),
        mcp_provenance_hash: Set(HASH_A.to_owned()),
        identity_hash: Set(HASH_B.to_owned()),
        payload_json: Set("{\"summary\":\"Safe\"}".to_owned()),
        provenance_json: Set("{\"schemaVersion\":1}".to_owned()),
        payload_size_bytes: Set(18),
        created_at: Set(now + time::Duration::seconds(2)),
    }
    .insert(&database)
    .await
    .expect("content artifact should insert");

    content_job_result::ActiveModel {
        job_id: Set(JOB_ID.to_owned()),
        artifact_id: Set(ARTIFACT_ID.to_owned()),
        was_reused: Set(false),
        linked_at: Set(now + time::Duration::seconds(2)),
    }
    .insert(&database)
    .await
    .expect("content job result should insert");

    let stored_job = content_job::Entity::find_by_id(JOB_ID)
        .one(&database)
        .await
        .expect("content job should query")
        .expect("content job should exist");
    assert_eq!(stored_job.target_locale, None);
    assert_eq!(stored_job.lease_token, 1);
    assert_eq!(
        stored_job.attempt_deadline_at,
        Some(now + time::Duration::seconds(180))
    );

    let stored_attempt = content_job_attempt::Entity::find_by_id(ATTEMPT_ID)
        .one(&database)
        .await
        .expect("attempt should query")
        .expect("attempt should exist");
    assert_eq!(
        stored_attempt.deadline_at,
        now + time::Duration::seconds(180)
    );
    assert_eq!(stored_attempt.estimated_cost_micros, 42);

    let stored_artifact = content_artifact::Entity::find_by_id(ARTIFACT_ID)
        .one(&database)
        .await
        .expect("artifact should query")
        .expect("artifact should exist");
    assert_eq!(stored_artifact.payload_json, "{\"summary\":\"Safe\"}");
    assert_eq!(stored_artifact.locale, None);

    let stored_result = content_job_result::Entity::find_by_id(JOB_ID)
        .one(&database)
        .await
        .expect("result should query")
        .expect("result should exist");
    assert_eq!(stored_result.artifact_id, ARTIFACT_ID);
    assert!(!stored_result.was_reused);

    rollback(&database).await.expect("rollback should succeed");
    assert_tables(&database, false).await;
    database.close().await.expect("database should close");
}

async fn assert_tables(database: &sea_orm::DatabaseConnection, expected: bool) {
    for table in [
        "content_jobs",
        "content_job_attempts",
        "content_artifacts",
        "content_job_results",
    ] {
        let row = database
            .query_one(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?",
                [table.into()],
            ))
            .await
            .expect("sqlite table catalog should query");
        assert_eq!(row.is_some(), expected, "table {table} presence");
    }
}

async fn assert_indexes(database: &sea_orm::DatabaseConnection) {
    let rows = database
        .query_all(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name LIKE '%content%'"
                .to_owned(),
        ))
        .await
        .expect("sqlite index catalog should query");
    let names = rows
        .into_iter()
        .map(index_name)
        .collect::<std::collections::HashSet<_>>();
    for expected in EXPECTED_INDEXES {
        assert!(names.contains(*expected), "missing index {expected}");
    }
}

fn index_name(row: QueryResult) -> String {
    String::try_get(&row, "", "name").expect("index name should decode")
}
