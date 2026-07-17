#[allow(dead_code)]
mod support;

use std::time::Duration;

use http::HeaderValue;
use raindrop::db::{
    entities::{entry, feed, feed_refresh_run, rss_counter},
    migrate, rollback,
};
use raindrop::feeds::{
    ClaimRequest, EncodedEntryContent, EntryContentDetail, EntryContentError, FeedParser,
    FeedRepository, FeedUrlPolicy, FetchOutcome, FetchedDocument, OpaqueValidator, PersistFeed,
    QueueRefreshRequest, RefreshRepositoryError, RefreshTrigger,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait,
    QueryFilter, QueryOrder, Statement,
};
use secrecy::SecretString;
use support::database::{FEED_ID, connect_for_contract, insert_feed};

static PARSER_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn parsed_content(source: &str) -> raindrop::content::SanitizedContent {
    let _guard = PARSER_MUTEX.lock().await;
    let url = FeedUrlPolicy::new(true)
        .normalize("https://example.test/feed.xml")
        .expect("fixture URL should normalize");
    let body = format!(
        "<rss version=\"2.0\"><channel><title>x</title><link>https://example.test/</link><item><guid>x</guid><description><![CDATA[{source}]]></description></item></channel></rss>"
    );
    let document = FetchedDocument::try_from(FetchOutcome::Document {
        url,
        document: body.into_bytes(),
        content_type: Some("application/rss+xml".to_owned()),
        etag: None,
        last_modified: None,
    })
    .expect("fixture should be a fetched document");
    let parsed = FeedParser::new()
        .parse(document)
        .await
        .expect("fixture should parse");
    parsed
        .entries()
        .first()
        .expect("fixture should contain one entry")
        .content()
        .clone()
}

#[tokio::test]
async fn content_storage_is_canonical_and_round_trips_inert_images() {
    let content = parsed_content(
        "<p class='publisher'>Hello</p><img src='/hero.jpg' alt='Hero' width='640' height='480'>",
    )
    .await;

    let encoded =
        EncodedEntryContent::from_sanitized(&content).expect("sanitized content should encode");
    assert_eq!(
        encoded.as_storage_str(),
        "rdsc:v1:{\"html\":\"<p>Hello</p><img alt=\\\"Hero\\\" width=\\\"640\\\" height=\\\"480\\\">\",\"inertImages\":[{\"imageIndex\":0,\"sourceUrl\":\"https://example.test/hero.jpg\",\"alt\":\"Hero\",\"width\":640,\"height\":480}]}"
    );

    let detail = EntryContentDetail::decode(encoded.as_storage_str())
        .expect("canonical envelope should decode");
    assert_eq!(detail.html(), content.html());
    assert_eq!(detail.inert_images(), content.images());
}

fn envelope(html: &str, inert_images: &str) -> String {
    format!(
        "rdsc:v1:{{\"html\":{},\"inertImages\":{inert_images}}}",
        serde_json::to_string(html).expect("test HTML should serialize")
    )
}

fn image(
    index: u32,
    source_url: &str,
    alt: Option<&str>,
    width: Option<u32>,
    height: Option<u32>,
) -> String {
    format!(
        "{{\"imageIndex\":{index},\"sourceUrl\":{},\"alt\":{},\"width\":{},\"height\":{}}}",
        serde_json::to_string(source_url).expect("test URL should serialize"),
        serde_json::to_string(&alt).expect("test alt should serialize"),
        width.map_or_else(|| "null".to_owned(), |value| value.to_string()),
        height.map_or_else(|| "null".to_owned(), |value| value.to_string()),
    )
}

#[test]
fn content_storage_rejects_untrusted_or_noncanonical_envelopes() {
    let matching = image(
        0,
        "https://img.example.test/a.jpg",
        Some("A"),
        Some(1),
        Some(2),
    );
    let valid_html = "<img alt=\"A\" width=\"1\" height=\"2\">";
    let cases = [
        "<p>legacy</p>".to_owned(),
        "rdsc:v2:{}".to_owned(),
        "rdsc:v1:not-json".to_owned(),
        format!(
            "rdsc:v1:{{ \"html\":{},\"inertImages\":[]}}",
            serde_json::to_string("<p>x</p>").unwrap()
        ),
        "rdsc:v1:{\"html\":\"<p>x</p>\",\"inertImages\":[],\"extra\":true}".to_owned(),
        envelope("<script>bad()</script><p>x</p>", "[]"),
        envelope("<p class=\"tracking\">x</p>", "[]"),
        envelope(valid_html, &format!("[{matching},{matching}]")),
        envelope(
            valid_html,
            &format!(
                "[{}]",
                image(
                    1,
                    "https://img.example.test/a.jpg",
                    Some("A"),
                    Some(1),
                    Some(2)
                )
            ),
        ),
        envelope(
            valid_html,
            &format!(
                "[{}]",
                image(0, "/relative.jpg", Some("A"), Some(1), Some(2))
            ),
        ),
        envelope(
            valid_html,
            &format!(
                "[{}]",
                image(
                    0,
                    "https://user:secret@img.example.test/a.jpg",
                    Some("A"),
                    Some(1),
                    Some(2)
                )
            ),
        ),
        envelope(
            valid_html,
            &format!(
                "[{}]",
                image(
                    0,
                    "https://img.example.test/a.jpg",
                    Some("different"),
                    Some(1),
                    Some(2)
                )
            ),
        ),
        envelope(
            valid_html,
            &format!(
                "[{}]",
                image(
                    0,
                    "https://img.example.test/a.jpg",
                    Some("A"),
                    Some(0),
                    Some(2)
                )
            ),
        ),
    ];

    for storage in cases {
        assert!(
            EntryContentDetail::decode(&storage).is_err(),
            "invalid storage should reject"
        );
    }
}

#[test]
fn content_storage_enforces_all_byte_and_count_budgets() {
    assert_eq!(
        EntryContentDetail::decode(&envelope(&"x".repeat(1024 * 1024 + 1), "[]")),
        Err(EntryContentError::HtmlTooLarge)
    );
    assert_eq!(
        EntryContentDetail::decode(&format!("rdsc:v1:{}", "x".repeat(4 * 1024 * 1024))),
        Err(EntryContentError::EnvelopeTooLarge)
    );

    let html = (0..257).map(|_| "<img>").collect::<String>();
    assert_eq!(
        EntryContentDetail::decode(&envelope(&html, "[]")),
        Err(EntryContentError::TooManyImages)
    );

    let metadata = (0..65)
        .map(|index| {
            image(
                index,
                &format!("https://img.example.test/{}/a.jpg", "x".repeat(4_000)),
                None,
                None,
                None,
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let html = (0..65).map(|_| "<img>").collect::<String>();
    assert_eq!(
        EntryContentDetail::decode(&envelope(&html, &format!("[{metadata}]"))),
        Err(EntryContentError::ImageMetadataTooLarge)
    );
}

#[test]
fn content_storage_errors_are_redacted() {
    let secret = "publisher-secret.example";
    let storage = envelope(
        "<img>",
        &format!(
            "[{}]",
            image(
                0,
                &format!("https://user:password@{secret}/x"),
                None,
                None,
                None
            )
        ),
    );
    let error = EntryContentDetail::decode(&storage).expect_err("credential URL should reject");
    assert!(!error.to_string().contains(secret));
    assert!(!format!("{error:?}").contains(secret));
}

const RSS_60: &[u8] = include_bytes!("fixtures/rss_2_60_items.xml");

async fn parsed_feed(body: impl Into<Vec<u8>>) -> raindrop::feeds::ParsedFeed {
    let _guard = PARSER_MUTEX.lock().await;
    let url = FeedUrlPolicy::new(true)
        .normalize("https://feeds.example.test/final.xml")
        .expect("fixture URL should normalize");
    let document = FetchedDocument::try_from(FetchOutcome::Document {
        url,
        document: body.into(),
        content_type: Some("application/rss+xml".to_owned()),
        etag: None,
        last_modified: None,
    })
    .expect("fixture should be a fetched document");
    FeedParser::new()
        .parse(document)
        .await
        .expect("fixture should parse")
}

async fn parsed_feed_with_validators(
    body: impl Into<Vec<u8>>,
    etag: OpaqueValidator,
    last_modified: OpaqueValidator,
) -> raindrop::feeds::ParsedFeed {
    let _guard = PARSER_MUTEX.lock().await;
    let url = FeedUrlPolicy::new(true)
        .normalize("https://feeds.example.test/final.xml")
        .expect("fixture URL should normalize");
    let document = FetchedDocument::try_from(FetchOutcome::Document {
        url,
        document: body.into(),
        content_type: Some("application/rss+xml".to_owned()),
        etag: Some(etag),
        last_modified: Some(last_modified),
    })
    .expect("fixture should be a fetched document");
    FeedParser::new()
        .parse(document)
        .await
        .expect("fixture should parse")
}

async fn sqlite_persistence_database(
    name: &str,
) -> (
    tempfile::TempDir,
    sea_orm::DatabaseConnection,
    FeedRepository,
) {
    let data = tempfile::tempdir().expect("temporary directory should be created");
    let url = format!(
        "sqlite://{}?mode=rwc",
        data.path().join(format!("{name}.db")).display()
    );
    let database = connect_for_contract(SecretString::from(url)).await;
    migrate(&database)
        .await
        .expect("entry persistence migrations should apply");
    insert_feed(&database, time::OffsetDateTime::now_utc()).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.entry_sequence_head = Set(0);
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.update(&database).await.expect("feed should unlock");
    let repository = FeedRepository::new(database.clone());
    (data, database, repository)
}

async fn claim_refresh(repository: &FeedRepository, key: &str) -> raindrop::feeds::RefreshClaim {
    repository
        .queue_refresh(QueueRefreshRequest {
            feed_id: FEED_ID.to_owned(),
            requested_by_user_id: None,
            trigger: RefreshTrigger::Manual,
            idempotency_key: key.to_owned(),
        })
        .await
        .expect("refresh should queue");
    repository
        .claim_due(ClaimRequest {
            owner: format!("worker-{key}"),
            lease_duration: Duration::from_secs(30),
        })
        .await
        .expect("refresh claim should not fail")
        .expect("refresh should claim")
}

#[tokio::test]
async fn sqlite_first_feed_persist_allocates_one_generation_and_monotonic_sequences() {
    let (_data, database, repository) = sqlite_persistence_database("first-feed-persist").await;
    let claim = claim_refresh(&repository, "first").await;
    let input = PersistFeed::try_from(parsed_feed(RSS_60).await)
        .expect("parsed feed should become owned persistence input");

    let result = repository
        .persist_feed(&claim, input)
        .await
        .expect("first feed should persist");
    assert_eq!(result.counts.new_count, 60);
    assert_eq!(result.counts.updated_count, 0);
    assert_eq!(result.counts.dropped_count, 0);
    assert_eq!(result.generation, Some(1));

    let rows = entry::Entity::find()
        .filter(entry::Column::FeedId.eq(FEED_ID))
        .order_by_asc(entry::Column::FeedSequence)
        .all(&database)
        .await
        .expect("persisted entries should query");
    assert_eq!(rows.len(), 60);
    assert!(rows.iter().all(|row| row.ingest_generation == 1));
    assert_eq!(
        rows.iter().map(|row| row.feed_sequence).collect::<Vec<_>>(),
        (1_i64..=60).collect::<Vec<_>>()
    );
    assert!(rows.iter().all(|row| row.identity_hash.len() == 64));
    assert!(rows.iter().all(|row| {
        row.identity_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }));

    let counter = rss_counter::Entity::find_by_id("INGEST_GENERATION")
        .one(&database)
        .await
        .expect("generation should query")
        .expect("generation should exist");
    assert_eq!(counter.value, 1);
    let feed = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    assert_eq!(feed.entry_sequence_head, 60);
    assert_eq!(feed.lease_owner, None);
    assert_eq!(feed.lease_until, None);
    let run = feed_refresh_run::Entity::find_by_id(&claim.run_id)
        .one(&database)
        .await
        .expect("refresh run should query")
        .expect("refresh run should exist");
    assert_eq!(run.status, "SUCCESS");
    assert_eq!(run.commit_generation, Some(1));
    assert_eq!(
        (run.new_count, run.updated_count, run.dropped_count),
        (60, 0, 0)
    );

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_identical_refresh_is_idempotent_and_does_not_increment_generation() {
    let (_data, database, repository) = sqlite_persistence_database("identical-refresh").await;
    let first_claim = claim_refresh(&repository, "identical-first").await;
    repository
        .persist_feed(
            &first_claim,
            PersistFeed::try_from(parsed_feed(RSS_60).await).expect("first input"),
        )
        .await
        .expect("first refresh should persist");
    let before = entry::Entity::find()
        .filter(entry::Column::FeedId.eq(FEED_ID))
        .order_by_asc(entry::Column::FeedSequence)
        .all(&database)
        .await
        .expect("first entries should query");

    let second_claim = claim_refresh(&repository, "identical-second").await;
    let result = repository
        .persist_feed(
            &second_claim,
            PersistFeed::try_from(parsed_feed(RSS_60).await).expect("second input"),
        )
        .await
        .expect("identical refresh should persist idempotently");
    assert_eq!(
        result.counts,
        raindrop::feeds::RefreshCounts {
            new_count: 0,
            updated_count: 0,
            dropped_count: 0,
        }
    );
    assert_eq!(result.generation, None);

    let after = entry::Entity::find()
        .filter(entry::Column::FeedId.eq(FEED_ID))
        .order_by_asc(entry::Column::FeedSequence)
        .all(&database)
        .await
        .expect("second entries should query");
    assert_eq!(after, before);
    let counter = rss_counter::Entity::find_by_id("INGEST_GENERATION")
        .one(&database)
        .await
        .expect("generation should query")
        .expect("generation should exist");
    assert_eq!(counter.value, 1);
    let run = feed_refresh_run::Entity::find_by_id(&second_claim.run_id)
        .one(&database)
        .await
        .expect("refresh run should query")
        .expect("refresh run should exist");
    assert_eq!(run.commit_generation, None);
    assert_eq!((run.new_count, run.updated_count), (0, 0));

    database.close().await.expect("database should close");
}

fn rss_item(description: &str) -> Vec<u8> {
    rss_guid_item("stable-guid", "Stable title", description)
}

fn rss_guid_item(guid: &str, title: &str, description: &str) -> Vec<u8> {
    format!(
        "<rss version=\"2.0\"><channel><title>x</title><link>https://example.test/</link><item><guid>{guid}</guid><title>{title}</title><link>https://example.test/entry</link><pubDate>16 Jul 2026 12:00:00 GMT</pubDate><description><![CDATA[{description}]]></description></item></channel></rss>"
    )
    .into_bytes()
}

async fn persist_one(
    repository: &FeedRepository,
    key: &str,
    description: &str,
) -> raindrop::feeds::PersistResult {
    let claim = claim_refresh(repository, key).await;
    repository
        .persist_feed(
            &claim,
            PersistFeed::try_from(parsed_feed(rss_item(description)).await)
                .expect("single entry should map"),
        )
        .await
        .expect("single entry should persist")
}

async fn only_entry(database: &sea_orm::DatabaseConnection) -> entry::Model {
    entry::Entity::find()
        .filter(entry::Column::FeedId.eq(FEED_ID))
        .one(database)
        .await
        .expect("entry should query")
        .expect("entry should exist")
}

#[tokio::test]
async fn sqlite_tracking_image_source_and_content_changes_have_distinct_update_semantics() {
    let (_data, database, repository) =
        sqlite_persistence_database("content-update-semantics").await;
    let tracking_a = parsed_feed(rss_item(
        "<p class='a' style='color:red' data-track='1'>same<img src='https://img.example.test/a.jpg' alt='A'></p>",
    ))
    .await;
    let tracking_b = parsed_feed(rss_item(
        "<p class='b' style='color:blue' data-track='2'>same<img src='https://img.example.test/a.jpg' alt='A'></p>",
    ))
    .await;
    assert_ne!(
        tracking_a.entries()[0].summary(),
        tracking_b.entries()[0].summary()
    );
    let first = persist_one(
        &repository,
        "content-first",
        "<p class='a' style='color:red' data-track='1'>same<img src='https://img.example.test/a.jpg' alt='A'></p>",
    )
    .await;
    assert_eq!((first.counts.new_count, first.counts.updated_count), (1, 0));
    let original = only_entry(&database).await;

    let tracking = persist_one(
        &repository,
        "content-tracking",
        "<p class='b' style='color:blue' data-track='2'>same<img src='https://img.example.test/a.jpg' alt='A'></p>",
    )
    .await;
    assert_eq!(
        (tracking.counts.new_count, tracking.counts.updated_count),
        (0, 0)
    );
    assert_eq!(only_entry(&database).await, original);

    let image_only = persist_one(
        &repository,
        "content-image-only",
        "<p class='c'>same<img src='https://img.example.test/b.jpg' alt='A'></p>",
    )
    .await;
    assert_eq!(
        (image_only.counts.new_count, image_only.counts.updated_count),
        (0, 1)
    );
    assert_eq!(image_only.generation, None);
    let image_changed = only_entry(&database).await;
    assert_eq!(image_changed.id, original.id);
    assert_eq!(image_changed.identity_kind, original.identity_kind);
    assert_eq!(image_changed.identity, original.identity);
    assert_eq!(image_changed.identity_hash, original.identity_hash);
    assert_eq!(image_changed.feed_sequence, original.feed_sequence);
    assert_eq!(image_changed.ingest_generation, original.ingest_generation);
    assert_eq!(image_changed.sort_at_us, original.sort_at_us);
    assert_eq!(image_changed.inserted_at, original.inserted_at);
    assert_eq!(
        image_changed.source_content_hash,
        original.source_content_hash
    );
    assert_eq!(image_changed.content_hash, original.content_hash);
    assert_ne!(image_changed.sanitized_content, original.sanitized_content);

    let content = persist_one(
        &repository,
        "content-real-change",
        "<p>different<img src='https://img.example.test/b.jpg' alt='A'></p>",
    )
    .await;
    assert_eq!(
        (content.counts.new_count, content.counts.updated_count),
        (0, 1)
    );
    assert_eq!(content.generation, None);
    let content_changed = only_entry(&database).await;
    assert_eq!(content_changed.id, original.id);
    assert_eq!(content_changed.identity, original.identity);
    assert_eq!(content_changed.feed_sequence, original.feed_sequence);
    assert_eq!(
        content_changed.ingest_generation,
        original.ingest_generation
    );
    assert_eq!(content_changed.sort_at_us, original.sort_at_us);
    assert_eq!(content_changed.inserted_at, original.inserted_at);
    assert_ne!(
        content_changed.source_content_hash,
        original.source_content_hash
    );
    assert_ne!(content_changed.content_hash, original.content_hash);

    database.close().await.expect("database should close");
}

fn fallback_rss(
    title: Option<&str>,
    published: Option<&str>,
    enclosure: bool,
    description: &str,
) -> Vec<u8> {
    let title = title.map_or_else(String::new, |value| format!("<title>{value}</title>"));
    let published =
        published.map_or_else(String::new, |value| format!("<pubDate>{value}</pubDate>"));
    let enclosure = if enclosure {
        "<enclosure url=\"https://cdn.example.test/audio.mp3\" type=\"audio/mpeg\" length=\"42\"/>"
    } else {
        ""
    };
    format!(
        "<rss version=\"2.0\"><channel><title>x</title><link>https://example.test/</link><item>{title}{published}{enclosure}<description><![CDATA[{description}]]></description></item></channel></rss>"
    )
    .into_bytes()
}

async fn persist_fallback(
    repository: &FeedRepository,
    key: &str,
    title: Option<&str>,
    published: Option<&str>,
    enclosure: bool,
    description: &str,
) -> raindrop::feeds::PersistResult {
    let claim = claim_refresh(repository, key).await;
    repository
        .persist_feed(
            &claim,
            PersistFeed::try_from(
                parsed_feed(fallback_rss(title, published, enclosure, description)).await,
            )
            .expect("fallback entry should map"),
        )
        .await
        .expect("fallback entry should persist")
}

#[tokio::test]
async fn sqlite_accepted_fallback_field_changes_allocate_distinct_identities() {
    let (_data, database, repository) = sqlite_persistence_database("fallback-identities").await;
    let fixtures = [
        ("fallback-content-a", None, None, false, "<p>one</p>"),
        ("fallback-content-b", None, None, false, "<p>two</p>"),
        (
            "fallback-title-a",
            Some("Title A"),
            None,
            false,
            "<p>two</p>",
        ),
        (
            "fallback-title-b",
            Some("Title B"),
            None,
            false,
            "<p>two</p>",
        ),
        (
            "fallback-date",
            Some("Title B"),
            Some("16 Jul 2026 12:00:00 GMT"),
            false,
            "<p>two</p>",
        ),
        (
            "fallback-enclosure",
            Some("Title B"),
            Some("16 Jul 2026 12:00:00 GMT"),
            true,
            "<p>two</p>",
        ),
    ];
    for (expected_generation, (key, title, published, enclosure, description)) in
        (1_i64..).zip(fixtures)
    {
        let result =
            persist_fallback(&repository, key, title, published, enclosure, description).await;
        assert_eq!(
            (result.counts.new_count, result.counts.updated_count),
            (1, 0)
        );
        assert_eq!(result.generation, Some(expected_generation));
    }

    let rows = entry::Entity::find()
        .filter(entry::Column::FeedId.eq(FEED_ID))
        .order_by_asc(entry::Column::FeedSequence)
        .all(&database)
        .await
        .expect("fallback entries should query");
    assert_eq!(rows.len(), 6);
    let identities = rows
        .iter()
        .map(|row| row.identity_hash.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(identities.len(), 6);
    assert_eq!(
        rows.iter().map(|row| row.feed_sequence).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5, 6]
    );

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_identity_hash_hits_compare_typed_kind_and_full_identity() {
    let (_data, database, repository) = sqlite_persistence_database("identity-collision").await;
    persist_one(&repository, "collision-first", "<p>first</p>").await;

    let incoming = parsed_feed(rss_guid_item(
        "different-guid",
        "Different",
        "<p>second</p>",
    ))
    .await;
    let incoming_identity = incoming.entries()[0].identity().clone();
    let input = PersistFeed::try_from(incoming).expect("collision input should map");
    let claim = claim_refresh(&repository, "collision-second").await;
    let existing = only_entry(&database).await;

    database
        .execute(Statement::from_sql_and_values(
            database.get_database_backend(),
            "UPDATE entries SET identity_kind=?, identity=?, identity_hash=? WHERE id=?",
            [
                "URL".into(),
                incoming_identity.identity().into(),
                incoming_identity.index_hash().into(),
                existing.id.as_str().into(),
            ],
        ))
        .await
        .expect("kind collision fixture should update");
    assert!(matches!(
        repository.persist_feed(&claim, input.clone()).await,
        Err(RefreshRepositoryError::IdentityHashCollision)
    ));

    database
        .execute(Statement::from_sql_and_values(
            database.get_database_backend(),
            "UPDATE entries SET identity_kind=?, identity=? WHERE id=?",
            [
                incoming_identity.kind().as_database_str().into(),
                "different-full-identity".into(),
                existing.id.as_str().into(),
            ],
        ))
        .await
        .expect("text collision fixture should update");
    assert!(matches!(
        repository.persist_feed(&claim, input).await,
        Err(RefreshRepositoryError::IdentityHashCollision)
    ));
    let counter = rss_counter::Entity::find_by_id("INGEST_GENERATION")
        .one(&database)
        .await
        .expect("generation should query")
        .expect("generation should exist");
    assert_eq!(counter.value, 1);
    assert_eq!(
        entry::Entity::find()
            .filter(entry::Column::FeedId.eq(FEED_ID))
            .all(&database)
            .await
            .expect("entries should query")
            .len(),
        1
    );

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_stale_token_writes_nothing() {
    let (_data, database, repository) = sqlite_persistence_database("stale-token").await;
    let claim = claim_refresh(&repository, "stale").await;
    let input = PersistFeed::try_from(parsed_feed(rss_item("<p>stale</p>")).await)
        .expect("stale input should map");
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.lease_token = Set(claim.lease_token + 1);
    active
        .update(&database)
        .await
        .expect("newer token fixture should update");

    assert!(matches!(
        repository.persist_feed(&claim, input).await,
        Err(RefreshRepositoryError::LeaseLost)
    ));
    assert!(
        entry::Entity::find()
            .filter(entry::Column::FeedId.eq(FEED_ID))
            .all(&database)
            .await
            .expect("entries should query")
            .is_empty()
    );
    let counter = rss_counter::Entity::find_by_id("INGEST_GENERATION")
        .one(&database)
        .await
        .expect("generation should query")
        .expect("generation should exist");
    assert_eq!(counter.value, 0);
    let run = feed_refresh_run::Entity::find_by_id(&claim.run_id)
        .one(&database)
        .await
        .expect("run should query")
        .expect("run should exist");
    assert_eq!(run.status, "RUNNING");
    assert_eq!(run.commit_generation, None);

    database.close().await.expect("database should close");
}

fn opaque(bytes: &[u8]) -> OpaqueValidator {
    OpaqueValidator::from_header(
        HeaderValue::from_bytes(bytes).expect("opaque validator fixture should be a header value"),
    )
    .expect("opaque validator fixture should be accepted")
}

async fn assert_validator_round_trip(
    database: &sea_orm::DatabaseConnection,
    repository: &FeedRepository,
    key: &str,
) {
    let etag_bytes = b"opaque-etag-\x80";
    let modified_bytes = b"opaque-modified-\xff";
    let claim = claim_refresh(repository, key).await;
    let parsed = parsed_feed_with_validators(
        rss_item("<p>validator</p>"),
        opaque(etag_bytes),
        opaque(modified_bytes),
    )
    .await;
    repository
        .persist_feed(
            &claim,
            PersistFeed::try_from(parsed).expect("validator input should map"),
        )
        .await
        .expect("validator input should persist");

    let validators = repository
        .load_validators(FEED_ID)
        .await
        .expect("stored validators should decode")
        .expect("validator URL should be stored");
    let request_url = FeedUrlPolicy::new(true)
        .normalize("https://feeds.example.test/final.xml")
        .expect("request URL should normalize");
    let reusable = validators
        .for_request(&request_url)
        .expect("validators should bind to the exact final URL");
    assert_eq!(reusable.etag().expect("etag").as_bytes(), etag_bytes);
    assert_eq!(
        reusable.last_modified().expect("last modified").as_bytes(),
        modified_bytes
    );

    let feed = feed::Entity::find_by_id(FEED_ID)
        .one(database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    assert_eq!(feed.fetch_url, "https://feeds.example.test/final.xml");
    assert_eq!(
        feed.validator_url.as_deref(),
        Some("https://feeds.example.test/final.xml")
    );
    assert!(
        feed.etag
            .as_deref()
            .is_some_and(|value| value.starts_with("v1:"))
    );
    assert!(
        feed.last_modified
            .as_deref()
            .is_some_and(|value| value.starts_with("v1:"))
    );
}

#[tokio::test]
async fn sqlite_non_utf8_validators_round_trip_and_corruption_is_redacted() {
    let (_data, database, repository) = sqlite_persistence_database("validator-roundtrip").await;
    assert_validator_round_trip(&database, &repository, "validator-sqlite").await;

    let secret = "https://secret.example.test/?token=do-not-log";
    database
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Sqlite,
            "UPDATE feeds SET etag=? WHERE id=?",
            [secret.into(), FEED_ID.into()],
        ))
        .await
        .expect("corrupt validator fixture should update");
    let error = repository
        .load_validators(FEED_ID)
        .await
        .expect_err("corrupt validator should fail closed");
    assert!(matches!(error, RefreshRepositoryError::CorruptValidator));
    assert!(!error.to_string().contains(secret));
    assert!(!format!("{error:?}").contains(secret));

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn postgres_entry_persistence_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_POSTGRES_URL") else {
        eprintln!(
            "postgres entry persistence contract skipped: test database URL is not configured"
        );
        return;
    };
    external_backend_persistence_contract(url, "validator-postgres").await;
}

#[tokio::test]
async fn mysql_entry_persistence_contract() {
    let Ok(url) = std::env::var("RAINDROP_TEST_MYSQL_URL") else {
        eprintln!("mysql entry persistence contract skipped: test database URL is not configured");
        return;
    };
    external_backend_persistence_contract(url, "validator-mysql").await;
}

async fn external_backend_persistence_contract(url: String, key: &str) {
    let database = connect_for_contract(SecretString::from(url)).await;
    rollback(&database)
        .await
        .expect("dedicated entry persistence database should reset");
    migrate(&database)
        .await
        .expect("entry persistence migrations should apply");
    insert_feed(&database, time::OffsetDateTime::now_utc()).await;
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.entry_sequence_head = Set(0);
    active.lease_owner = Set(None);
    active.lease_until = Set(None);
    active.update(&database).await.expect("feed should unlock");
    let repository = FeedRepository::new(database.clone());

    assert_validator_round_trip(&database, &repository, key).await;
    let identical = persist_one(&repository, "backend-identical", "<p>validator</p>").await;
    assert_eq!(
        (identical.counts.new_count, identical.counts.updated_count),
        (0, 0)
    );
    assert_eq!(identical.generation, None);

    let content = persist_one(
        &repository,
        "backend-content",
        "<p>same<img src='https://img.example.test/a.jpg' alt='A'></p>",
    )
    .await;
    assert_eq!(
        (content.counts.new_count, content.counts.updated_count),
        (0, 1)
    );
    let tracking = persist_one(
        &repository,
        "backend-tracking",
        "<p class='tracking' style='color:red'>same<img src='https://img.example.test/a.jpg' alt='A'></p>",
    )
    .await;
    assert_eq!(
        (tracking.counts.new_count, tracking.counts.updated_count),
        (0, 0)
    );
    let image = persist_one(
        &repository,
        "backend-image",
        "<p>same<img src='https://img.example.test/b.jpg' alt='A'></p>",
    )
    .await;
    assert_eq!((image.counts.new_count, image.counts.updated_count), (0, 1));

    let metadata_claim = claim_refresh(&repository, "backend-metadata").await;
    let metadata = repository
        .persist_feed(
            &metadata_claim,
            PersistFeed::try_from(
                parsed_feed(rss_guid_item(
                    "stable-guid",
                    "Updated backend title",
                    "<p>same<img src='https://img.example.test/b.jpg' alt='A'></p>",
                ))
                .await,
            )
            .expect("backend metadata input should map"),
        )
        .await
        .expect("backend metadata input should persist");
    assert_eq!(
        (metadata.counts.new_count, metadata.counts.updated_count),
        (0, 1)
    );

    let fallback = persist_fallback(
        &repository,
        "backend-fallback",
        None,
        None,
        false,
        "<p>fallback</p>",
    )
    .await;
    assert_eq!(fallback.generation, Some(2));

    let concurrent_claim = claim_refresh(&repository, "backend-concurrent").await;
    let first_input = PersistFeed::try_from(
        parsed_feed(rss_guid_item(
            "backend-concurrent-guid",
            "Concurrent",
            "<p>concurrent</p>",
        ))
        .await,
    )
    .expect("first backend concurrent input should map");
    let second_input = PersistFeed::try_from(
        parsed_feed(rss_guid_item(
            "backend-concurrent-guid",
            "Concurrent",
            "<p>concurrent</p>",
        ))
        .await,
    )
    .expect("second backend concurrent input should map");
    let competing_repository = FeedRepository::new(database.clone());
    let (first, second) = tokio::join!(
        repository.persist_feed(&concurrent_claim, first_input),
        competing_repository.persist_feed(&concurrent_claim, second_input)
    );
    assert_eq!(
        [&first, &second]
            .into_iter()
            .filter(|result| result.is_ok())
            .count(),
        1
    );
    assert_eq!(
        [&first, &second]
            .into_iter()
            .filter(|result| matches!(result, Err(RefreshRepositoryError::LeaseLost)))
            .count(),
        1
    );

    let stale_claim = claim_refresh(&repository, "backend-stale").await;
    let stale_input = PersistFeed::try_from(
        parsed_feed(rss_guid_item("backend-stale-guid", "Stale", "<p>stale</p>")).await,
    )
    .expect("backend stale input should map");
    let model = feed::Entity::find_by_id(FEED_ID)
        .one(&database)
        .await
        .expect("feed should query")
        .expect("feed should exist");
    let mut active: feed::ActiveModel = model.into();
    active.lease_token = Set(stale_claim.lease_token + 1);
    active
        .update(&database)
        .await
        .expect("backend newer token fixture should update");
    assert!(matches!(
        repository.persist_feed(&stale_claim, stale_input).await,
        Err(RefreshRepositoryError::LeaseLost)
    ));

    let rows = entry::Entity::find()
        .filter(entry::Column::FeedId.eq(FEED_ID))
        .all(&database)
        .await
        .expect("entry should query");
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rss_counter::Entity::find_by_id("INGEST_GENERATION")
            .one(&database)
            .await
            .expect("generation should query")
            .expect("generation should exist")
            .value,
        3
    );

    rollback(&database)
        .await
        .expect("dedicated entry persistence database should roll back");
    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_concurrent_persists_leave_one_identity_row() {
    let (_data, database, repository) = sqlite_persistence_database("concurrent-persist").await;
    let competing_repository = FeedRepository::new(database.clone());
    let claim = claim_refresh(&repository, "concurrent").await;
    let first_input = PersistFeed::try_from(parsed_feed(rss_item("<p>concurrent</p>")).await)
        .expect("first concurrent input should map");
    let second_input = PersistFeed::try_from(parsed_feed(rss_item("<p>concurrent</p>")).await)
        .expect("second concurrent input should map");

    let (first, second) = tokio::join!(
        repository.persist_feed(&claim, first_input),
        competing_repository.persist_feed(&claim, second_input)
    );
    let successes = [&first, &second]
        .into_iter()
        .filter(|result| result.is_ok())
        .count();
    let lease_losses = [&first, &second]
        .into_iter()
        .filter(|result| matches!(result, Err(RefreshRepositoryError::LeaseLost)))
        .count();
    assert_eq!(successes, 1);
    assert_eq!(lease_losses, 1);
    assert_eq!(
        entry::Entity::find()
            .filter(entry::Column::FeedId.eq(FEED_ID))
            .all(&database)
            .await
            .expect("entries should query")
            .len(),
        1
    );
    let counter = rss_counter::Entity::find_by_id("INGEST_GENERATION")
        .one(&database)
        .await
        .expect("generation should query")
        .expect("generation should exist");
    assert_eq!(counter.value, 1);

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_stable_guid_metadata_change_updates_existing_row() {
    let (_data, database, repository) = sqlite_persistence_database("stable-metadata-update").await;
    let first_claim = claim_refresh(&repository, "metadata-first").await;
    repository
        .persist_feed(
            &first_claim,
            PersistFeed::try_from(
                parsed_feed(rss_guid_item("metadata-guid", "First title", "<p>same</p>")).await,
            )
            .expect("first metadata input should map"),
        )
        .await
        .expect("first metadata input should persist");
    let before = only_entry(&database).await;

    let second_claim = claim_refresh(&repository, "metadata-second").await;
    let result = repository
        .persist_feed(
            &second_claim,
            PersistFeed::try_from(
                parsed_feed(rss_guid_item(
                    "metadata-guid",
                    "Second title",
                    "<p>same</p>",
                ))
                .await,
            )
            .expect("second metadata input should map"),
        )
        .await
        .expect("metadata change should persist");
    assert_eq!(
        (result.counts.new_count, result.counts.updated_count),
        (0, 1)
    );
    assert_eq!(result.generation, None);
    let after = only_entry(&database).await;
    assert_eq!(after.title.as_deref(), Some("Second title"));
    assert_eq!(after.id, before.id);
    assert_eq!(after.identity, before.identity);
    assert_eq!(after.identity_hash, before.identity_hash);
    assert_eq!(after.feed_sequence, before.feed_sequence);
    assert_eq!(after.ingest_generation, before.ingest_generation);
    assert_eq!(after.sort_at_us, before.sort_at_us);
    assert_eq!(after.inserted_at, before.inserted_at);
    assert_eq!(after.sanitized_content, before.sanitized_content);
    assert_eq!(after.source_content_hash, before.source_content_hash);
    assert_eq!(after.content_hash, before.content_hash);

    database.close().await.expect("database should close");
}

#[tokio::test]
async fn sqlite_sort_keys_accept_pre_epoch_and_post_2038_dates_and_use_db_time_fallback() {
    let (_data, database, repository) = sqlite_persistence_database("sort-key-boundaries").await;
    let body = br#"<rss version="2.0"><channel><title>x</title><link>https://example.test/</link>
        <item><guid>pre</guid><title>pre</title><pubDate>31 Dec 1969 23:59:59 GMT</pubDate><description>pre</description></item>
        <item><guid>post</guid><title>post</title><pubDate>03 Feb 2040 04:05:06 GMT</pubDate><description>post</description></item>
        <item><guid>fallback</guid><title>fallback</title><description>fallback</description></item>
        </channel></rss>"#;
    let claim = claim_refresh(&repository, "sort-boundaries").await;
    repository
        .persist_feed(
            &claim,
            PersistFeed::try_from(parsed_feed(body.as_slice()).await)
                .expect("sort boundary input should map"),
        )
        .await
        .expect("sort boundary input should persist");

    let rows = entry::Entity::find()
        .filter(entry::Column::FeedId.eq(FEED_ID))
        .order_by_asc(entry::Column::FeedSequence)
        .all(&database)
        .await
        .expect("sort boundary entries should query");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].published_at_us, Some(-1_000_000));
    assert_eq!(rows[0].sort_at_us, -1_000_000);
    assert_eq!(rows[1].published_at_us, Some(2_211_854_706_000_000));
    assert_eq!(rows[1].sort_at_us, 2_211_854_706_000_000);
    assert_eq!(rows[2].published_at_us, None);
    assert!(rows[2].sort_at_us > 1_700_000_000_000_000);
    assert!(rows[2].sort_at_us < 2_211_854_706_000_000);

    database.close().await.expect("database should close");
}
