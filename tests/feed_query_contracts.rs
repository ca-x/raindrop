#[allow(dead_code)]
mod support;

use raindrop::db::{entities::feed, migrate, rollback};
use raindrop::feeds::{EntryListState, FeedRepository, ListEntriesQuery};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection};
use secrecy::SecretString;
use time::OffsetDateTime;
use uuid::Uuid;

use support::database::{
    FEED_ID, USER_A_ID, connect_for_contract, entry_model, insert_feed, insert_user,
    subscription_model,
};

#[tokio::test]
async fn postgres_entry_query_explain_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!("skipping PostgreSQL entry query EXPLAIN; RAINDROP_TEST_POSTGRES_URL is unset");
        return;
    };
    explain_contract(SecretString::from(url), "postgres").await;
}

#[tokio::test]
async fn mysql_entry_query_explain_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("skipping MySQL entry query EXPLAIN; RAINDROP_TEST_MYSQL_URL is unset");
        return;
    };
    explain_contract(SecretString::from(url), "mysql").await;
}

async fn explain_contract(url: SecretString, backend_name: &str) {
    let database = connect_for_contract(url).await;
    let _ = rollback(&database).await;
    migrate(&database)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} migrations should apply"));
    seed_noise(&database).await;
    let statistics = match backend_name {
        "postgres" => "ANALYZE",
        "mysql" => "ANALYZE TABLE subscriptions, feeds, entries, entry_states",
        _ => unreachable!("conditional contracts cover PostgreSQL and MySQL only"),
    };
    database
        .execute_unprepared(statistics)
        .await
        .unwrap_or_else(|_| panic!("{backend_name} statistics should collect"));
    let repository = FeedRepository::new(database.clone());

    for state in [EntryListState::All, EntryListState::Unread] {
        for feed_id in [None, Some(FEED_ID.to_owned())] {
            let plan = repository
                .explain_list_for_user(
                    USER_A_ID,
                    ListEntriesQuery {
                        state,
                        feed_id,
                        ..ListEntriesQuery::default()
                    },
                )
                .await
                .unwrap_or_else(|_| panic!("{backend_name} EXPLAIN should execute"));
            let joined = plan.join("\n");
            assert!(
                joined.contains("uq_subscriptions_user_feed")
                    || joined.contains("idx_subscriptions_user_pos"),
                "{backend_name} must use a user-leading subscription index: {joined}"
            );
            assert!(
                joined.contains("idx_entries_feed_list")
                    || joined.contains("uq_entries_feed_seq")
                    || joined.contains("idx_entries_snapshot"),
                "{backend_name} must use a feed-leading entry index: {joined}"
            );
        }
    }
    database.close().await.expect("database should close");
}

async fn seed_noise(database: &DatabaseConnection) {
    let now = OffsetDateTime::now_utc();
    insert_user(database, USER_A_ID, "target-reader").await;
    insert_feed(database, now).await;
    let mut target = subscription_model(&Uuid::new_v4().to_string(), USER_A_ID, now);
    target.start_sequence = Set(0);
    target.read_through_sequence = Set(0);
    target.insert(database).await.expect("target subscription");

    for index in 0..16_u128 {
        let user_id =
            Uuid::from_u128(0x2000_0000_0000_4000_8000_0000_0000_0000 + index).to_string();
        insert_user(database, &user_id, &format!("noise-{index:03}")).await;
        let noise_feed_id = Uuid::new_v4().to_string();
        let noise_url = format!("https://noise-{index:03}.example.test/feed.xml");
        feed::ActiveModel {
            id: Set(noise_feed_id.clone()),
            source_url: Set(noise_url.clone()),
            normalized_url: Set(noise_url.clone()),
            normalized_url_hash: Set(blake3::hash(noise_url.as_bytes()).to_hex().to_string()),
            fetch_url: Set(noise_url),
            title: Set(Some(format!("Noise feed {index:03}"))),
            site_url: Set(None),
            validator_url: Set(None),
            etag: Set(None),
            last_modified: Set(None),
            response_content_hash: Set(None),
            entry_sequence_head: Set(64),
            last_attempt_at: Set(None),
            last_success_at: Set(None),
            last_changed_at: Set(None),
            next_fetch_at: Set(now),
            retry_after_at: Set(None),
            consecutive_failures: Set(0),
            last_error_code: Set(None),
            is_disabled: Set(false),
            orphaned_at: Set(None),
            lease_owner: Set(None),
            lease_token: Set(0),
            lease_until: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(database)
        .await
        .expect("noise feed");
        let mut subscription = subscription_model(&Uuid::new_v4().to_string(), &user_id, now);
        subscription.feed_id = Set(noise_feed_id.clone());
        subscription.start_sequence = Set(0);
        subscription.read_through_sequence = Set(0);
        subscription
            .insert(database)
            .await
            .expect("noise subscription");
        for sequence in 1..=64_i64 {
            let identity = format!("noise-{index}-{sequence}");
            let identity_hash = blake3::hash(identity.as_bytes()).to_hex().to_string();
            let mut model = entry_model(
                &Uuid::new_v4().to_string(),
                sequence,
                &identity,
                &identity_hash,
                Some(1_000_000_000_000_000 + sequence),
                now,
            );
            model.feed_id = Set(noise_feed_id.clone());
            model.insert(database).await.expect("noise entry");
        }
    }
    for sequence in 1..=512_i64 {
        let identity = format!("query-contract-{sequence}");
        let identity_hash = blake3::hash(identity.as_bytes()).to_hex().to_string();
        entry_model(
            &Uuid::new_v4().to_string(),
            sequence,
            &identity,
            &identity_hash,
            Some(2_000_000_000_000_000 + sequence),
            now,
        )
        .insert(database)
        .await
        .expect("noise entry");
    }
}
