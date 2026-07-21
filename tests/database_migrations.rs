use raindrop::db::{
    DatabaseConfig, connect,
    entities::{bootstrap_state, session, user, user_role},
    migrate, rollback,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, Statement,
};
use secrecy::SecretString;
use tempfile::tempdir;
use time::OffsetDateTime;
use uuid::Uuid;

#[tokio::test]
async fn sqlite_identity_migrations_are_idempotent_and_enforce_constraints() {
    let data = tempdir().expect("temporary directory should be created");
    let database_path = data.path().join("raindrop.db");
    let url = format!("sqlite://{}?mode=rwc", database_path.display());
    let database = connect(&DatabaseConfig::new(SecretString::from(url)))
        .await
        .expect("SQLite should connect");

    migrate(&database).await.expect("migration should pass");
    migrate(&database)
        .await
        .expect("migration should be idempotent");

    let journal_mode = database
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "PRAGMA journal_mode".to_owned(),
        ))
        .await
        .expect("journal mode query should pass")
        .expect("journal mode row should exist")
        .try_get::<String>("", "journal_mode")
        .expect("journal mode should be text");
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

    let foreign_keys = database
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "PRAGMA foreign_keys".to_owned(),
        ))
        .await
        .expect("foreign key query should pass")
        .expect("foreign key row should exist")
        .try_get::<i64>("", "foreign_keys")
        .expect("foreign key flag should be numeric");
    assert_eq!(foreign_keys, 1);

    let now = OffsetDateTime::now_utc();
    let user_id = Uuid::new_v4().to_string();
    user::ActiveModel {
        id: Set(user_id.clone()),
        username: Set("Reader".to_owned()),
        normalized_username: Set("reader".to_owned()),
        display_name: Set(None),
        email: Set(None),
        password_hash: Set("test-hash".to_owned()),
        is_disabled: Set(false),
        created_at: Set(now),
        last_login_at: Set(None),
    }
    .insert(&database)
    .await
    .expect("user should insert");

    let duplicate_username = user::ActiveModel {
        id: Set(Uuid::new_v4().to_string()),
        username: Set("reader".to_owned()),
        normalized_username: Set("reader".to_owned()),
        display_name: Set(None),
        email: Set(None),
        password_hash: Set("test-hash".to_owned()),
        is_disabled: Set(false),
        created_at: Set(now),
        last_login_at: Set(None),
    }
    .insert(&database)
    .await;
    assert!(duplicate_username.is_err());

    let role = user_role::ActiveModel {
        user_id: Set(user_id.clone()),
        role: Set("ADMIN".to_owned()),
    };
    role.clone()
        .insert(&database)
        .await
        .expect("role should insert");
    assert!(role.insert(&database).await.is_err());

    session::ActiveModel {
        token_hash: Set("session-token-hash".to_owned()),
        user_id: Set(user_id.clone()),
        csrf_hash: Set("csrf-token-hash".to_owned()),
        created_at: Set(now),
        last_seen_at: Set(now),
        expires_at: Set(now + time::Duration::hours(1)),
    }
    .insert(&database)
    .await
    .expect("session should insert");

    user::Entity::delete_by_id(&user_id)
        .exec(&database)
        .await
        .expect("user should delete");

    assert_eq!(
        user_role::Entity::find()
            .filter(user_role::Column::UserId.eq(&user_id))
            .count(&database)
            .await
            .expect("roles should count"),
        0
    );
    assert_eq!(
        session::Entity::find()
            .filter(session::Column::UserId.eq(&user_id))
            .count(&database)
            .await
            .expect("sessions should count"),
        0
    );
}

#[tokio::test]
async fn migrations_support_up_down_up_and_restore_foreign_key_behavior() {
    let data = tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("rollback.db").display()
    );
    let database = connect(&DatabaseConfig::new(SecretString::from(url)))
        .await
        .expect("SQLite should connect");

    migrate(&database).await.expect("migration up should pass");
    rollback(&database)
        .await
        .expect("migration down should pass");
    migrate(&database)
        .await
        .expect("migration recreation should pass");

    let now = OffsetDateTime::now_utc();
    let user_id = Uuid::new_v4().to_string();
    user::ActiveModel {
        id: Set(user_id.clone()),
        username: Set("Reader".to_owned()),
        normalized_username: Set("reader".to_owned()),
        display_name: Set(None),
        email: Set(None),
        password_hash: Set("test-hash".to_owned()),
        is_disabled: Set(false),
        created_at: Set(now),
        last_login_at: Set(None),
    }
    .insert(&database)
    .await
    .expect("user should insert after recreation");
    user_role::ActiveModel {
        user_id: Set(user_id.clone()),
        role: Set("ADMIN".to_owned()),
    }
    .insert(&database)
    .await
    .expect("role should insert after recreation");
    user::Entity::delete_by_id(&user_id)
        .exec(&database)
        .await
        .expect("user should delete");
    assert_eq!(
        user_role::Entity::find()
            .filter(user_role::Column::UserId.eq(&user_id))
            .count(&database)
            .await
            .expect("roles should count"),
        0
    );
}

#[tokio::test]
async fn bootstrap_state_migration_backfills_an_existing_user_claim() {
    let data = tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("bootstrap-backfill.db").display()
    );
    let database = connect(&DatabaseConfig::new(SecretString::from(url)))
        .await
        .expect("SQLite should connect");
    migrate(&database).await.expect("migration should pass");

    let user_id = Uuid::new_v4().to_string();
    user::ActiveModel {
        id: Set(user_id.clone()),
        username: Set("Existing".to_owned()),
        normalized_username: Set("existing".to_owned()),
        display_name: Set(None),
        email: Set(None),
        password_hash: Set("test-hash".to_owned()),
        is_disabled: Set(false),
        created_at: Set(OffsetDateTime::now_utc()),
        last_login_at: Set(None),
    }
    .insert(&database)
    .await
    .expect("existing user should insert");
    database
        .execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "DROP TABLE bootstrap_state".to_owned(),
        ))
        .await
        .expect("bootstrap table should drop");
    database
        .execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "DELETE FROM seaql_migrations WHERE version = 'bootstrap_state'".to_owned(),
        ))
        .await
        .expect("bootstrap migration marker should clear");

    migrate(&database)
        .await
        .expect("bootstrap migration should reapply");
    let claim = bootstrap_state::Entity::find_by_id(1)
        .one(&database)
        .await
        .expect("claim should load")
        .expect("claim should exist");
    assert_eq!(claim.administrator_user_id, user_id);
}
