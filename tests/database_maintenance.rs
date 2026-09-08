#[allow(dead_code)]
mod support;

use raindrop::db::{DatabaseConfig, connect, connect_reader, maintenance, migrate};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use secrecy::SecretString;

#[tokio::test]
async fn compact_reclaims_deleted_pages_and_preserves_readable_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("maintenance.db");
    let config = DatabaseConfig::new(SecretString::from(format!(
        "sqlite://{}?mode=rwc",
        path.display()
    )));
    let db = connect(&config).await.unwrap();
    migrate(&db).await.unwrap();
    let reader = connect_reader(&config, &db).await.unwrap();
    for sql in [
        "CREATE TABLE storage_fixture (id INTEGER PRIMARY KEY, payload BLOB)",
        "INSERT INTO storage_fixture VALUES (1, randomblob(8388608)), (2, 'keep me')",
        "PRAGMA wal_checkpoint(TRUNCATE)",
        "DELETE FROM storage_fixture WHERE id=1",
    ] {
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .unwrap();
    }
    let before = maintenance::storage(&reader).await.unwrap();
    assert!(before.free_bytes > 8_000_000);
    let result = maintenance::compact(&db).await.unwrap();
    assert!(result.reclaimed_bytes > 8_000_000);
    assert_eq!(result.after.free_bytes, 0);
    assert!(result.wal_truncated);
    assert_eq!(result.after.wal_bytes, 0);
    let row = reader
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "SELECT payload FROM storage_fixture WHERE id=2".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "payload").unwrap(), "keep me");
    let check = reader
        .query_one(Statement::from_string(
            DatabaseBackend::Sqlite,
            "PRAGMA integrity_check".to_owned(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        check.try_get::<String>("", "integrity_check").unwrap(),
        "ok"
    );
    let again = maintenance::compact(&db).await.unwrap();
    assert_eq!(again.reclaimed_bytes, 0);
}

#[tokio::test]
async fn in_memory_database_is_rejected_without_mutation() {
    let db = connect(&DatabaseConfig::new(SecretString::from("sqlite::memory:")))
        .await
        .unwrap();
    assert!(matches!(
        maintenance::compact(&db).await,
        Err(maintenance::MaintenanceError::Unsupported)
    ));
}

#[tokio::test]
async fn held_reader_does_not_block_optional_checkpoint_or_fake_disk_reclamation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("held-reader.db");
    let config = DatabaseConfig::new(SecretString::from(format!(
        "sqlite://{}?mode=rwc",
        path.display()
    )));
    let db = connect(&config).await.unwrap();
    migrate(&db).await.unwrap();
    for sql in [
        "CREATE TABLE storage_fixture (id INTEGER PRIMARY KEY, payload BLOB)",
        "INSERT INTO storage_fixture VALUES (1, randomblob(8388608)), (2, 'keep me')",
        "PRAGMA wal_checkpoint(TRUNCATE)",
        "DELETE FROM storage_fixture WHERE id=1",
        "PRAGMA wal_checkpoint(TRUNCATE)",
    ] {
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .unwrap();
    }
    let reader = connect_reader(&config, &db).await.unwrap();
    let mut held = reader.get_sqlite_connection_pool().acquire().await.unwrap();
    sea_orm::sqlx::query("BEGIN")
        .execute(&mut *held)
        .await
        .unwrap();
    sea_orm::sqlx::query("SELECT * FROM storage_fixture")
        .fetch_all(&mut *held)
        .await
        .unwrap();
    let before = maintenance::storage(&db).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), maintenance::compact(&db))
        .await
        .expect("optional checkpoint must not wait five seconds")
        .unwrap();
    assert!(!result.wal_truncated);
    let main_bytes = std::fs::metadata(&path).unwrap().len();
    let wal_bytes = std::fs::metadata(format!("{}-wal", path.display()))
        .unwrap()
        .len();
    assert_eq!(result.after.database_bytes, main_bytes);
    assert_eq!(result.after.wal_bytes, wal_bytes);
    assert_eq!(
        result.reclaimed_bytes,
        (before.database_bytes + before.wal_bytes).saturating_sub(main_bytes + wal_bytes)
    );
    let timeout: i64 = sea_orm::sqlx::query_scalar("PRAGMA busy_timeout")
        .fetch_one(db.get_sqlite_connection_pool())
        .await
        .unwrap();
    assert_eq!(timeout, 5000);
    sea_orm::sqlx::query("ROLLBACK")
        .execute(&mut *held)
        .await
        .unwrap();
    drop(held);
    assert!(maintenance::compact(&db).await.unwrap().wal_truncated);
}

#[tokio::test]
async fn retention_child_lookups_use_entry_indexes_instead_of_global_scans() {
    let db = connect(&DatabaseConfig::new(SecretString::from("sqlite::memory:")))
        .await
        .unwrap();
    migrate(&db).await.unwrap();
    for (table, index) in [
        ("entry_states", "idx_states_entry_retention"),
        ("content_jobs", "idx_jobs_entry_retention"),
        ("content_artifacts", "idx_artifacts_entry_retention"),
    ] {
        let plans = db
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!("EXPLAIN QUERY PLAN SELECT 1 FROM {table} WHERE entry_id='candidate'"),
            ))
            .await
            .unwrap();
        let details = plans
            .iter()
            .map(|row| row.try_get::<String>("", "detail").unwrap())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(details.contains(index), "{details}");
        assert!(!details.contains(&format!("SCAN {table}")), "{details}");
    }
}
