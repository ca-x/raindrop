use raindrop::{
    auth::{
        AuthenticateError, CreateAdminError, CreateAdminInput, LoginIdentifier, PasswordService,
        authenticate, create_admin,
    },
    db::{DatabaseConfig, connect, entities::user, migrate},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use secrecy::SecretString;
use tempfile::tempdir;

#[tokio::test]
async fn admin_creation_and_authentication_are_secure() {
    let data = tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("auth.db").display()
    );
    let database = connect(&DatabaseConfig::new(SecretString::from(url)))
        .await
        .expect("database should connect");
    migrate(&database).await.expect("database should migrate");
    let passwords = PasswordService::default();

    let created = create_admin(
        &database,
        &passwords,
        CreateAdminInput {
            username: "Reader".to_owned(),
            password: SecretString::from("correct horse battery staple".to_owned()),
            email: Some("reader@example.com".to_owned()),
        },
    )
    .await
    .expect("admin should be created");

    assert_eq!(created.username, "Reader");
    assert!(created.is_admin());
    let public_json = serde_json::to_string(&created).expect("user should serialize");
    assert!(!public_json.contains("password"));
    assert!(!public_json.contains("hash"));

    let authenticated = authenticate(
        &database,
        &passwords,
        LoginIdentifier::new(" reader "),
        &SecretString::from("correct horse battery staple".to_owned()),
    )
    .await
    .expect("valid credentials should authenticate");
    assert_eq!(authenticated.id, created.id);

    let duplicate = create_admin(
        &database,
        &passwords,
        CreateAdminInput {
            username: "READER".to_owned(),
            password: SecretString::from("another secure password".to_owned()),
            email: None,
        },
    )
    .await;
    assert!(matches!(duplicate, Err(CreateAdminError::UsernameTaken)));

    let wrong_password = authenticate(
        &database,
        &passwords,
        LoginIdentifier::new("reader"),
        &SecretString::from("wrong password value".to_owned()),
    )
    .await;
    assert!(matches!(
        wrong_password,
        Err(AuthenticateError::InvalidCredentials)
    ));

    let mut active: user::ActiveModel = user::Entity::find()
        .filter(user::Column::Id.eq(&created.id))
        .one(&database)
        .await
        .expect("user lookup should work")
        .expect("user should exist")
        .into();
    active.is_disabled = Set(true);
    sea_orm::ActiveModelTrait::update(active, &database)
        .await
        .expect("user should be disabled");

    let disabled = authenticate(
        &database,
        &passwords,
        LoginIdentifier::new("reader@example.com"),
        &SecretString::from("correct horse battery staple".to_owned()),
    )
    .await;
    assert!(matches!(disabled, Err(AuthenticateError::Disabled)));
}
