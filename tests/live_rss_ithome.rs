#[allow(dead_code)]
mod support;

use std::{collections::HashSet, time::Duration};

use raindrop::db::{entities::entry, migrate};
use raindrop::feeds::{
    ClaimRequest, EntryListState, ExactClaimResult, FeedCommandService, FeedExecutor,
    FeedRepository, FeedUrlPolicy, HttpFeedTransport, ListEntriesQuery, QueueSubscriptionRefresh,
    RefreshStatus, SubscribeInput, install_ring_crypto_provider,
};
use sea_orm::{EntityTrait, PaginatorTrait, QuerySelect};
use secrecy::SecretString;

use support::database::{USER_A_ID, connect_for_contract, insert_user};

#[tokio::test]
#[ignore = "requires RAINDROP_LIVE_RSS_SMOKE=1 and public network"]
async fn ithome_feed_securely_ingests_and_deduplicates() {
    if std::env::var("RAINDROP_LIVE_RSS_SMOKE").as_deref() != Ok("1") {
        panic!("set RAINDROP_LIVE_RSS_SMOKE=1 to run the live RSS smoke");
    }

    install_ring_crypto_provider().expect("production TLS provider should install");
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join("live-rss-ithome.db").display()
    );
    let database = connect_for_contract(SecretString::from(database_url)).await;
    migrate(&database)
        .await
        .expect("RSS migrations should apply");
    insert_user(&database, USER_A_ID, "live-reader").await;

    let repository = FeedRepository::new(database.clone());
    let policy = FeedUrlPolicy::new(false);
    let (transport, execution_counter) =
        HttpFeedTransport::new_observed(policy, 2).expect("production transport should build");
    let command = FeedCommandService::new(repository.clone(), policy);
    let executor = FeedExecutor::new(repository.clone(), policy, transport);
    let subscription = command
        .subscribe(
            USER_A_ID,
            SubscribeInput {
                url: "https://www.ithome.com/rss/".to_owned(),
            },
        )
        .await
        .expect("live subscription should queue");
    let first_run_id = subscription
        .subscription
        .refresh
        .as_ref()
        .expect("new live feed should queue a refresh")
        .run_id
        .clone();
    let ExactClaimResult::Claimed(first_claim) = repository
        .claim_run(
            &first_run_id,
            ClaimRequest {
                owner: "live-ithome-first".to_owned(),
                lease_duration: Duration::from_secs(60),
            },
        )
        .await
        .expect("first live refresh should claim exactly")
    else {
        panic!("first live refresh should be claimed");
    };
    let first = executor
        .execute_claim(first_claim)
        .await
        .expect("first live refresh should execute");
    assert!(
        matches!(
            first.status,
            RefreshStatus::Success | RefreshStatus::Partial
        ),
        "first representation must parse and persist securely"
    );
    assert!(
        (50..=100).contains(&first.new_count),
        "first representation must contain a bounded realistic item count"
    );

    let mut cursor = None;
    let mut items = Vec::new();
    loop {
        let page = repository
            .list_for_user(
                USER_A_ID,
                ListEntriesQuery {
                    state: EntryListState::All,
                    limit: 100,
                    cursor,
                    ..ListEntriesQuery::default()
                },
            )
            .await
            .expect("live list should remain user visible");
        items.extend(page.items);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert!((50..=100).contains(&items.len()));
    let unique_ids = items
        .iter()
        .map(|item| item.entry_id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(unique_ids.len(), items.len(), "entry IDs must be unique");

    for item in &items {
        let detail = repository
            .get_detail_for_user(USER_A_ID, &item.entry_id)
            .await
            .expect("live detail query should remain typed")
            .expect("every live detail should be visible through the subscription");
        assert_safe_html(&detail.content_html);
    }

    let before_count = entry::Entity::find()
        .count(&database)
        .await
        .expect("entry count should query");
    let before_identities = entry::Entity::find()
        .select_only()
        .column(entry::Column::IdentityHash)
        .into_tuple::<String>()
        .all(&database)
        .await
        .expect("entry identities should query");
    assert_eq!(
        before_identities.iter().collect::<HashSet<_>>().len(),
        before_identities.len(),
        "persisted identities must be unique"
    );

    tokio::time::sleep(Duration::from_secs(31)).await;
    let second_queued = command
        .queue_subscription_refresh(
            USER_A_ID,
            &subscription.subscription.subscription_id,
            QueueSubscriptionRefresh {
                request_id: "00000000-0000-4000-8000-000000000501".to_owned(),
            },
        )
        .await
        .expect("manual live refresh should queue after cooldown");
    let ExactClaimResult::Claimed(second_claim) = repository
        .claim_run(
            &second_queued.run_id,
            ClaimRequest {
                owner: "live-ithome-second".to_owned(),
                lease_duration: Duration::from_secs(60),
            },
        )
        .await
        .expect("second live refresh should claim exactly")
    else {
        panic!("second live refresh should be claimed");
    };
    let second = executor
        .execute_claim(second_claim)
        .await
        .expect("manual live refresh should execute");
    assert!(
        matches!(
            second.status,
            RefreshStatus::NotModified | RefreshStatus::Success | RefreshStatus::Partial
        ),
        "second request must be 304 or a deduplicated 200"
    );
    assert_eq!(
        second.new_count, 0,
        "second request must not duplicate entries"
    );
    let after_count = entry::Entity::find()
        .count(&database)
        .await
        .expect("entry count should query");
    assert_eq!(after_count, before_count);
    assert_eq!(
        execution_counter.count(),
        2,
        "two executor refreshes must execute at most two feed HTTP requests including redirects"
    );

    eprintln!(
        "live RSS observation date={} count={} first_status={} second_status={} second_http={:?}",
        time::OffsetDateTime::now_utc().date(),
        items.len(),
        first.status,
        second.status,
        second.http_status
    );
}

fn assert_safe_html(html: &str) {
    let lower = html.to_ascii_lowercase();
    for forbidden in [
        "<script",
        "<style",
        "<iframe",
        "<form",
        "<svg",
        " onload=",
        " onclick=",
        " onerror=",
        " src=",
        "srcset=",
        "poster=",
        "class=",
        "data-",
        "style=",
    ] {
        assert!(!lower.contains(forbidden), "unsafe HTML token {forbidden}");
    }
}
