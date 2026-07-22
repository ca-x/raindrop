#[allow(dead_code)]
mod support;

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use raindrop::{
    backups::{
        BackupJobStatus, BackupPublicConfig, BackupRepository, BackupSecretConfig,
        CreateBackupTarget, RetentionPolicy, S3PublicConfig, S3SecretConfig, WebDavPublicConfig,
        WebDavSecretConfig,
    },
    content::provider::ProviderSecretKeyring,
    db::{
        entities::{backup_schedule, backup_target},
        migrate,
    },
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use secrecy::SecretString;
use support::database::{USER_A_ID, USER_B_ID, connect_for_contract, insert_user};
use tempfile::tempdir;
use time::{Duration, OffsetDateTime};

#[tokio::test]
async fn multiple_s3_and_webdav_targets_share_jobs_but_keep_isolated_results() {
    let data = tempdir().expect("temporary directory");
    let database = connect_for_contract(SecretString::from(format!(
        "sqlite://{}?mode=rwc",
        data.path().join("backups.db").display()
    )))
    .await;
    migrate(&database).await.expect("backup migrations");
    insert_user(&database, USER_A_ID, "backup-a").await;
    insert_user(&database, USER_B_ID, "backup-b").await;
    let repository = BackupRepository::new(database.clone(), Some(Arc::new(keyring())));

    let s3 = repository
        .create_target(
            USER_A_ID,
            CreateBackupTarget {
                display_name: "Primary S3".to_owned(),
                enabled: true,
                config: BackupPublicConfig::S3(S3PublicConfig {
                    endpoint: "https://objects.example".to_owned(),
                    region: "us-east-1".to_owned(),
                    bucket: "reader-backups".to_owned(),
                    prefix: "daily".to_owned(),
                    path_style: true,
                }),
                secret: BackupSecretConfig::S3(S3SecretConfig {
                    access_key_id: SecretString::from("access-sentinel"),
                    secret_access_key: SecretString::from("secret-sentinel"),
                    session_token: None,
                }),
                retention: RetentionPolicy {
                    retain_count: Some(7),
                    retain_days: Some(30),
                },
            },
        )
        .await
        .expect("S3 target should save");
    let webdav = repository
        .create_target(
            USER_A_ID,
            CreateBackupTarget {
                display_name: "Home WebDAV".to_owned(),
                enabled: true,
                config: BackupPublicConfig::Webdav(WebDavPublicConfig {
                    endpoint: "https://dav.example/storage".to_owned(),
                    prefix: "rss".to_owned(),
                }),
                secret: BackupSecretConfig::Webdav(WebDavSecretConfig {
                    username: SecretString::from("reader"),
                    password: SecretString::from("password-sentinel"),
                }),
                retention: RetentionPolicy::default(),
            },
        )
        .await
        .expect("WebDAV target should save");

    let stored = backup_target::Entity::find_by_id(&s3.target_id)
        .one(&database)
        .await
        .expect("target query")
        .expect("target row");
    assert!(
        stored
            .secret_config_ciphertext
            .starts_with("rdsec1.backup.")
    );
    assert!(!stored.secret_config_ciphertext.contains("access-sentinel"));
    assert!(!stored.secret_config_ciphertext.contains("secret-sentinel"));

    let target_ids = vec![s3.target_id.clone(), webdav.target_id.clone()];
    let schedule = repository
        .put_schedule(USER_A_ID, true, 12, &target_ids)
        .await
        .expect("multi-target schedule should save");
    assert_eq!(schedule.target_ids.len(), 2);

    let queued = repository
        .enqueue_manual(USER_A_ID, &target_ids)
        .await
        .expect("manual backup should queue");
    assert_eq!(queued.target_count, 2);
    let claim = repository
        .claim_next("backup-test-worker")
        .await
        .expect("claim query")
        .expect("job should claim");
    let targets = repository
        .pending_execution_targets(&claim)
        .await
        .expect("both targets should decrypt");
    assert_eq!(targets.len(), 2);
    assert!(
        targets
            .iter()
            .all(|target| target.object_key.ends_with(".opml"))
    );

    repository
        .mark_target_running(&claim, &targets[0].target_result_id)
        .await
        .expect("first target running");
    repository
        .complete_target(&claim, &targets[0].target_result_id, Ok(512))
        .await
        .expect("first target succeeds");
    repository
        .mark_target_running(&claim, &targets[1].target_result_id)
        .await
        .expect("second target running");
    repository
        .complete_target(
            &claim,
            &targets[1].target_result_id,
            Err(raindrop::backups::BackupError::new(
                raindrop::backups::BackupErrorKind::TargetUnreachable,
            )),
        )
        .await
        .expect("second target failure should persist");
    let completed = repository
        .finish_job(&claim)
        .await
        .expect("job should finish");
    assert_eq!(completed.status, BackupJobStatus::Partial);
    assert_eq!(
        completed
            .targets
            .iter()
            .filter(|target| target.byte_size == Some(512))
            .count(),
        1
    );
    assert_eq!(
        completed
            .targets
            .iter()
            .filter(|target| target.error_code.as_deref() == Some("TARGET_UNREACHABLE"))
            .count(),
        1
    );

    assert!(
        repository
            .list_targets(USER_B_ID)
            .await
            .expect("other user list")
            .is_empty()
    );
    assert!(
        repository
            .enqueue_manual(USER_B_ID, &target_ids)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn due_schedule_enqueues_once_and_advances_its_slot() {
    let data = tempdir().expect("temporary directory");
    let database = connect_for_contract(SecretString::from(format!(
        "sqlite://{}?mode=rwc",
        data.path().join("schedule.db").display()
    )))
    .await;
    migrate(&database).await.expect("backup migrations");
    insert_user(&database, USER_A_ID, "backup-schedule").await;
    let repository = BackupRepository::new(database.clone(), Some(Arc::new(keyring())));
    let target = repository
        .create_target(
            USER_A_ID,
            CreateBackupTarget {
                display_name: "Scheduled WebDAV".to_owned(),
                enabled: true,
                config: BackupPublicConfig::Webdav(WebDavPublicConfig {
                    endpoint: "https://dav.example".to_owned(),
                    prefix: String::new(),
                }),
                secret: BackupSecretConfig::Webdav(WebDavSecretConfig {
                    username: SecretString::from("reader"),
                    password: SecretString::from("password"),
                }),
                retention: RetentionPolicy::default(),
            },
        )
        .await
        .expect("target should save");
    repository
        .put_schedule(USER_A_ID, true, 24, &[target.target_id])
        .await
        .expect("schedule should save");
    let stored = backup_schedule::Entity::find_by_id(USER_A_ID)
        .one(&database)
        .await
        .expect("schedule query")
        .expect("schedule row");
    let original_slot = OffsetDateTime::now_utc() - Duration::hours(48);
    let mut active: backup_schedule::ActiveModel = stored.into();
    active.next_run_at = Set(Some(original_slot));
    active.update(&database).await.expect("force schedule due");

    assert_eq!(
        repository
            .enqueue_due_schedules()
            .await
            .expect("first due pass"),
        1
    );
    assert_eq!(
        repository
            .enqueue_due_schedules()
            .await
            .expect("second due pass"),
        0
    );
    let jobs = repository
        .list_jobs(USER_A_ID, None, 10)
        .await
        .expect("history should query");
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0].trigger_kind,
        raindrop::backups::BackupTriggerKind::Scheduled
    );
    let advanced = backup_schedule::Entity::find_by_id(USER_A_ID)
        .one(&database)
        .await
        .expect("advanced query")
        .expect("advanced row");
    assert!(
        advanced
            .next_run_at
            .is_some_and(|next| next > OffsetDateTime::now_utc())
    );
}

fn keyring() -> ProviderSecretKeyring {
    ProviderSecretKeyring::from_entries(&[SecretString::from(format!(
        "backup:{}",
        URL_SAFE_NO_PAD.encode([42_u8; 32])
    ))])
    .expect("backup keyring")
}
