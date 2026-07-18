#[allow(dead_code)]
mod support;

use raindrop::{
    db::{
        DatabaseConfig, connect,
        entities::{category, subscription},
        migrate,
    },
    organization::{CategoryError, CategoryRepository, CreateCategory, UpdateCategory},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait};
use secrecy::SecretString;
use support::database::{
    SUBSCRIPTION_A_ID, USER_A_ID, USER_B_ID, insert_feed, insert_user, subscription_model,
};
use tempfile::TempDir;
use time::OffsetDateTime;

const CATEGORY_A_ID: &str = "00000000-0000-4000-8000-000000000501";

#[tokio::test]
async fn create_normalizes_titles_and_orders_by_position_then_id() {
    let fixture = RepositoryFixture::new().await;
    let technology = fixture
        .repository
        .create(
            USER_A_ID,
            CreateCategory {
                title: "  Technology  ".to_owned(),
            },
        )
        .await
        .expect("technology category should create");
    let science = fixture
        .repository
        .create(
            USER_A_ID,
            CreateCategory {
                title: "Science".to_owned(),
            },
        )
        .await
        .expect("science category should create");

    assert_eq!(technology.title, "Technology");
    assert_eq!(technology.position, 1024);
    assert_eq!(science.position, 2048);

    fixture
        .repository
        .update(
            USER_A_ID,
            &science.category_id,
            UpdateCategory {
                title: None,
                position: Some(512),
            },
        )
        .await
        .expect("science position should update");
    let listed = fixture
        .repository
        .list(USER_A_ID)
        .await
        .expect("categories should list");
    assert_eq!(
        listed
            .iter()
            .map(|category| category.title.as_str())
            .collect::<Vec<_>>(),
        ["Science", "Technology"]
    );
}

#[tokio::test]
async fn title_validation_is_deterministic_and_redacted() {
    let fixture = RepositoryFixture::new().await;
    for title in [
        "".to_owned(),
        " ".to_owned(),
        "a".repeat(81),
        "界".repeat(80),
        "bad\nname".to_owned(),
        "name\n".to_owned(),
        "bad\u{0085}name".to_owned(),
    ] {
        let error = fixture
            .repository
            .create(
                USER_A_ID,
                CreateCategory {
                    title: title.clone(),
                },
            )
            .await
            .expect_err("invalid category title should fail");
        assert_eq!(error, CategoryError::InvalidTitle);
        if !title.is_empty() {
            assert!(!format!("{error:?}").contains(&title));
        }
    }
}

#[tokio::test]
async fn normalized_title_is_unique_only_inside_one_user_scope() {
    let fixture = RepositoryFixture::new().await;
    fixture
        .repository
        .create(
            USER_A_ID,
            CreateCategory {
                title: "Technology".to_owned(),
            },
        )
        .await
        .expect("first category should create");
    let duplicate = fixture
        .repository
        .create(
            USER_A_ID,
            CreateCategory {
                title: "  TECHNOLOGY  ".to_owned(),
            },
        )
        .await
        .expect_err("normalized duplicate should fail");
    assert_eq!(duplicate, CategoryError::Conflict);

    fixture
        .repository
        .create(
            USER_B_ID,
            CreateCategory {
                title: "TECHNOLOGY".to_owned(),
            },
        )
        .await
        .expect("another user should own the same normalized title");
}

#[tokio::test]
async fn update_and_delete_hide_cross_user_category_ids() {
    let fixture = RepositoryFixture::new().await;
    let category = fixture
        .repository
        .create(
            USER_A_ID,
            CreateCategory {
                title: "Private".to_owned(),
            },
        )
        .await
        .expect("private category should create");

    assert_eq!(
        fixture
            .repository
            .update(
                USER_B_ID,
                &category.category_id,
                UpdateCategory {
                    title: Some("Stolen".to_owned()),
                    position: None,
                },
            )
            .await
            .expect_err("other user update should be hidden"),
        CategoryError::NotFound
    );
    assert_eq!(
        fixture
            .repository
            .delete(USER_B_ID, &category.category_id)
            .await
            .expect_err("other user delete should be hidden"),
        CategoryError::NotFound
    );
    assert_eq!(fixture.repository.list(USER_B_ID).await.unwrap(), []);
}

#[tokio::test]
async fn update_rejects_empty_or_negative_patches() {
    let fixture = RepositoryFixture::new().await;
    assert_eq!(
        fixture
            .repository
            .update(USER_A_ID, CATEGORY_A_ID, UpdateCategory::default())
            .await
            .expect_err("empty patch should fail"),
        CategoryError::InvalidPatch
    );
    assert_eq!(
        fixture
            .repository
            .update(
                USER_A_ID,
                CATEGORY_A_ID,
                UpdateCategory {
                    title: None,
                    position: Some(-1),
                },
            )
            .await
            .expect_err("negative position should fail"),
        CategoryError::InvalidPosition
    );
}

#[tokio::test]
async fn category_quota_is_checked_under_the_user_lock() {
    let fixture = RepositoryFixture::new().await;
    let now = OffsetDateTime::now_utc();
    for index in 0..250_u16 {
        category::ActiveModel {
            id: Set(format!("10000000-0000-4000-8000-{index:012}")),
            user_id: Set(USER_A_ID.to_owned()),
            title: Set(format!("Category {index}")),
            normalized_title: Set(format!("category {index}")),
            position: Set(i64::from(index) * 1024),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&fixture.database)
        .await
        .expect("quota fixture category should insert");
    }

    assert_eq!(
        fixture
            .repository
            .create(
                USER_A_ID,
                CreateCategory {
                    title: "Over quota".to_owned(),
                },
            )
            .await
            .expect_err("category quota should reject the next create"),
        CategoryError::Limit
    );
}

#[tokio::test]
async fn concurrent_duplicate_create_has_one_winner_and_one_conflict() {
    let fixture = RepositoryFixture::new().await;
    let first = fixture.repository.clone();
    let second = fixture.repository.clone();
    let input = CreateCategory {
        title: "Concurrent".to_owned(),
    };
    let (left, right) = tokio::join!(
        first.create(USER_A_ID, input.clone()),
        second.create(USER_A_ID, input),
    );
    let results = [left, right];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(CategoryError::Conflict)))
            .count(),
        1
    );
    assert_eq!(fixture.repository.list(USER_A_ID).await.unwrap().len(), 1);
}

#[tokio::test]
async fn delete_uncategorizes_subscription_without_deleting_it() {
    let fixture = RepositoryFixture::new().await;
    insert_feed(&fixture.database, OffsetDateTime::now_utc()).await;
    let category = fixture
        .repository
        .create(
            USER_A_ID,
            CreateCategory {
                title: "Keep feed".to_owned(),
            },
        )
        .await
        .expect("category should create");
    let mut subscription =
        subscription_model(SUBSCRIPTION_A_ID, USER_A_ID, OffsetDateTime::now_utc());
    subscription.category_id = Set(Some(category.category_id.clone()));
    subscription
        .insert(&fixture.database)
        .await
        .expect("categorized subscription should insert");

    fixture
        .repository
        .delete(USER_A_ID, &category.category_id)
        .await
        .expect("category should delete");
    let stored = subscription::Entity::find_by_id(SUBSCRIPTION_A_ID)
        .one(&fixture.database)
        .await
        .expect("subscription should query")
        .expect("subscription should survive category deletion");
    assert_eq!(stored.category_id, None);
}

struct RepositoryFixture {
    _data: TempDir,
    database: DatabaseConnection,
    repository: CategoryRepository,
}

impl RepositoryFixture {
    async fn new() -> Self {
        let data = tempfile::tempdir().expect("temporary directory should be created");
        let url = format!(
            "sqlite://{}?mode=rwc",
            data.path().join("category-repository.db").display()
        );
        let database = connect(&DatabaseConfig::new(SecretString::from(url)))
            .await
            .expect("category repository database should connect");
        migrate(&database)
            .await
            .expect("category repository database should migrate");
        insert_user(&database, USER_A_ID, "category-user-a").await;
        insert_user(&database, USER_B_ID, "category-user-b").await;
        let repository = CategoryRepository::new(database.clone());
        Self {
            _data: data,
            database,
            repository,
        }
    }
}
