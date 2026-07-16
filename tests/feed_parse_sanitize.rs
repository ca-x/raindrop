use std::cell::RefCell;

use html5ever::{
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, StartTag, TagToken, Token, TokenSink, TokenSinkResult, Tokenizer,
        TokenizerOpts,
    },
};
use http::HeaderValue;
use raindrop::{
    content::{SanitizeError, sanitize_entry_html},
    feeds::{
        FeedParseError, FeedParseErrorKind, FeedParser, FeedUrlPolicy, FetchOutcome,
        FetchedDocument, FetchedDocumentError, IdentityKind, OpaqueValidator, ParsedFeed,
        ParsedFeedVersion,
    },
};

static PARSER_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const RSS_60: &[u8] = include_bytes!("fixtures/rss_2_60_items.xml");
const ATOM: &[u8] = include_bytes!("fixtures/atom_mixed.xml");
const RDF: &[u8] = include_bytes!("fixtures/rss_1_rdf.xml");
const JSON_FEED: &[u8] = include_bytes!("fixtures/json_feed.json");
const WINDOWS_1252: &[u8] = include_bytes!("fixtures/rss_windows_1252.xml");
const IDENTITY_EDGES: &[u8] = include_bytes!("fixtures/rss_identity_edges.xml");
const MALICIOUS_HTML: &[u8] = include_bytes!("fixtures/malicious_html.xml");
const UNSAFE_XML: &[u8] = include_bytes!("fixtures/unsafe_xml.xml");

fn fetched(body: impl Into<Vec<u8>>, content_type: Option<&str>) -> FetchedDocument {
    fetched_at(
        "https://feeds.example.test/source.xml?secret=redact-me",
        body,
        content_type,
    )
}

fn fetched_at(
    final_url: &str,
    body: impl Into<Vec<u8>>,
    content_type: Option<&str>,
) -> FetchedDocument {
    let url = FeedUrlPolicy::new(true)
        .normalize(final_url)
        .expect("fixture URL is valid");
    FetchedDocument::try_from(FetchOutcome::Document {
        url,
        document: body.into(),
        content_type: content_type.map(str::to_owned),
        etag: None,
        last_modified: None,
    })
    .expect("document outcome is accepted")
}

async fn parse(document: FetchedDocument) -> Result<ParsedFeed, FeedParseError> {
    let _guard = PARSER_MUTEX.lock().await;
    FeedParser::new().parse(document).await
}

async fn parse_fixture(body: &[u8], content_type: &str) -> ParsedFeed {
    parse(fetched(body, Some(content_type)))
        .await
        .expect("fixture parses")
}

#[test]
fn not_modified_never_enters_the_parser() {
    let url = FeedUrlPolicy::new(true)
        .normalize("https://example.test/feed.xml")
        .expect("URL");
    let error = FetchedDocument::try_from(FetchOutcome::NotModified {
        url,
        etag: None,
        last_modified: None,
    })
    .expect_err("304 is not a parser document");
    assert_eq!(error, FetchedDocumentError::NotDocument);
}

#[tokio::test]
async fn rss_2_fixture_has_sixty_stable_owned_entries() {
    let parsed = parse_fixture(RSS_60, "application/rss+xml; charset=utf-8").await;
    assert_eq!(parsed.version(), ParsedFeedVersion::Rss20);
    assert_eq!(parsed.title(), Some("Raindrop deterministic sixty"));
    assert_eq!(parsed.entries().len(), 60);
    assert_eq!(parsed.duplicate_count(), 0);
    for (index, entry) in parsed.entries().iter().enumerate() {
        assert_eq!(entry.identity().kind(), IdentityKind::Guid);
        assert_eq!(
            entry.identity().identity(),
            format!("fixture-guid-{:02}", index + 1)
        );
        assert!(entry.canonical_url().is_some_and(|url| !url.contains('#')));
        assert!(!entry.content().html().contains("src="));
        assert_eq!(entry.content().images().len(), 1);
        assert_eq!(entry.content().images()[0].image_index(), 0);
    }
}

#[tokio::test]
async fn atom_xml_base_and_published_updated_precedence_are_stable() {
    let parsed = parse(fetched_at(
        "https://transport.example.test/original.atom",
        ATOM,
        Some("application/atom+xml"),
    ))
    .await
    .expect("Atom parses");
    assert_eq!(parsed.version(), ParsedFeedVersion::Atom10);
    assert_eq!(
        parsed.canonical_url(),
        Some("https://atom.example.test/home")
    );
    assert_eq!(parsed.entries().len(), 2);
    let first = &parsed.entries()[0];
    assert_eq!(
        first.canonical_url(),
        Some("https://atom.example.test/root/posts/one")
    );
    assert_eq!(
        first.published_at().map(|date| date.unix_timestamp()),
        Some(1_784_203_200)
    );
    assert_eq!(
        first.content().images()[0].source_url(),
        "https://atom.example.test/root/assets/one.jpg"
    );
    let second = &parsed.entries()[1];
    assert_eq!(
        second.published_at().map(|date| date.unix_timestamp()),
        Some(1_784_206_800)
    );
    assert!(
        second
            .content()
            .html()
            .contains("&lt;b&gt;literal markup&lt;/b&gt;")
    );
}

#[tokio::test]
async fn rdf_and_json_feed_share_the_owned_domain_contract() {
    let rdf = parse_fixture(RDF, "application/rdf+xml").await;
    assert_eq!(rdf.version(), ParsedFeedVersion::Rss10);
    assert_eq!(rdf.entries().len(), 1);
    assert_eq!(rdf.entries()[0].identity().kind(), IdentityKind::Url);

    let json = parse_fixture(JSON_FEED, "application/feed+json").await;
    assert_eq!(json.version(), ParsedFeedVersion::JsonFeed11);
    assert_eq!(json.entries().len(), 1);
    assert_eq!(json.entries()[0].identity().identity(), "json-entry-one");
    assert_eq!(json.entries()[0].enclosures().len(), 1);
    assert_eq!(
        json.entries()[0].enclosures()[0].url(),
        "https://cdn.example.test/audio.mp3"
    );
}

#[tokio::test]
async fn rss_091_userland_passes_while_netscape_doctype_rejects() {
    let userland = br#"<rss version="0.91"><channel><title>x</title><link>https://example.test/</link><description>x</description></channel></rss>"#;
    let parsed = parse_fixture(userland, "application/rss+xml").await;
    assert_eq!(parsed.version(), ParsedFeedVersion::Rss091Userland);

    let error = parse(fetched(UNSAFE_XML, Some("application/rss+xml")))
        .await
        .expect_err("DTD-bearing feed rejects");
    assert_eq!(error.kind(), FeedParseErrorKind::DoctypeForbidden);
}

#[tokio::test]
async fn windows_1252_declaration_decodes_once_without_mojibake() {
    let parsed = parse_fixture(WINDOWS_1252, "application/rss+xml").await;
    assert_eq!(parsed.source().original_encoding(), "windows-1252");
    assert_eq!(parsed.entries()[0].title(), Some("Price €10 — café"));
    assert!(
        parsed.entries()[0]
            .content()
            .html()
            .contains("Smart “quotes” and em dash —.")
    );
}

#[tokio::test]
async fn mime_sniff_mismatch_and_hard_deny_matrix_is_fail_closed() {
    assert_eq!(
        parse(fetched(RSS_60, None))
            .await
            .expect("missing MIME sniffs")
            .entries()
            .len(),
        60
    );
    for tolerated in ["text/plain", "text/html", "application/octet-stream"] {
        assert_eq!(
            parse(fetched(RSS_60, Some(tolerated)))
                .await
                .expect("tolerated MIME sniffs")
                .entries()
                .len(),
            60
        );
    }
    let mismatch = parse(fetched(RSS_60, Some("application/json")))
        .await
        .expect_err("direct JSON MIME cannot carry XML");
    assert_eq!(mismatch.kind(), FeedParseErrorKind::MimeMismatch);
    for denied in ["image/png", "audio/mpeg", "video/mp4", "application/pdf"] {
        let error = parse(fetched(RSS_60, Some(denied)))
            .await
            .expect_err("hard-deny MIME rejects feed-shaped body");
        assert_eq!(error.kind(), FeedParseErrorKind::UnsupportedContentType);
    }
}

#[tokio::test]
async fn strict_charset_failures_are_typed() {
    let unknown = parse(fetched(
        RSS_60,
        Some("application/rss+xml; charset=x-secret-charset"),
    ))
    .await
    .expect_err("unknown charset rejects");
    assert_eq!(unknown.kind(), FeedParseErrorKind::UnsupportedCharset);

    let duplicate = parse(fetched(
        RSS_60,
        Some("application/rss+xml; charset=utf-8; charset=windows-1252"),
    ))
    .await
    .expect_err("duplicate charset rejects");
    assert_eq!(duplicate.kind(), FeedParseErrorKind::UnsupportedContentType);

    let utf32 = parse(fetched(
        [vec![0x00, 0x00, 0xfe, 0xff], RSS_60.to_vec()].concat(),
        Some("application/rss+xml"),
    ))
    .await
    .expect_err("UTF-32 rejects");
    assert_eq!(utf32.kind(), FeedParseErrorKind::UnsupportedCharset);

    let lossy = parse(fetched(
        b"<rss version=\"2.0\"><channel><title>\xff</title></channel></rss>".to_vec(),
        Some("application/rss+xml; charset=utf-8"),
    ))
    .await
    .expect_err("lossy UTF-8 rejects");
    assert_eq!(lossy.kind(), FeedParseErrorKind::DecodeFailed);
}

#[tokio::test]
async fn bom_http_charset_and_xml_declaration_precedence_is_exact() {
    let bom_body = [
        vec![0xef, 0xbb, 0xbf],
        "<?xml version=\"1.0\" encoding=\"windows-1252\"?><rss version=\"2.0\"><channel><title>UTF-8 café</title><link>https://example.test/</link><description>x</description></channel></rss>"
            .as_bytes()
            .to_vec(),
    ]
    .concat();
    let bom = parse(fetched(
        bom_body,
        Some("application/rss+xml; charset=windows-1252"),
    ))
    .await
    .expect("BOM wins");
    assert_eq!(bom.source().original_encoding(), "utf-8");
    assert_eq!(bom.title(), Some("UTF-8 café"));

    let source = "<?xml version=\"1.0\" encoding=\"utf-8\"?><rss version=\"2.0\"><channel><title>HTTP café — €</title><link>https://example.test/</link><description>x</description></channel></rss>";
    let (encoded, _, had_errors) = encoding_rs::WINDOWS_1252.encode(source);
    assert!(!had_errors);
    let http = parse(fetched(
        encoded.into_owned(),
        Some("application/rss+xml; charset=windows-1252"),
    ))
    .await
    .expect("HTTP charset wins");
    assert_eq!(http.source().original_encoding(), "windows-1252");
    assert_eq!(http.title(), Some("HTTP café — €"));

    let replacement = parse(fetched(
        "<rss version=\"2.0\"><channel><title>\u{fffd}</title></channel></rss>",
        Some("application/rss+xml; charset=utf-8"),
    ))
    .await
    .expect_err("conversion may not emit replacement character");
    assert_eq!(replacement.kind(), FeedParseErrorKind::DecodeFailed);
}

#[tokio::test]
async fn malformed_declarations_mime_and_conversion_expansion_reject() {
    for body in [
        b"<?xml encoding=\"utf-8\"?><rss version=\"2.0\"></rss>".as_slice(),
        b"<?xml version=\"1.1\"?><rss version=\"2.0\"></rss>".as_slice(),
        b"<?xml version=\"1.0\" encoding=\"utf-8\" encoding=\"utf-8\"?><rss version=\"2.0\"></rss>"
            .as_slice(),
    ] {
        let error = parse(fetched(body, Some("application/rss+xml")))
            .await
            .expect_err("malformed declaration rejects");
        assert_eq!(error.kind(), FeedParseErrorKind::MalformedXml);
    }
    let malformed_mime = parse(fetched(RSS_60, Some("application/rss+xml; charset")))
        .await
        .expect_err("malformed MIME rejects");
    assert_eq!(
        malformed_mime.kind(),
        FeedParseErrorKind::UnsupportedContentType
    );

    let mut expanded = b"<?xml version=\"1.0\"?><rss version=\"2.0\"><channel><title>".to_vec();
    expanded.extend(std::iter::repeat_n(0x80, 4 * 1024 * 1024));
    expanded.extend_from_slice(b"</title></channel></rss>");
    let error = parse(fetched(
        expanded,
        Some("application/rss+xml; charset=windows-1252"),
    ))
    .await
    .expect_err("post-conversion expansion rejects");
    assert_eq!(error.kind(), FeedParseErrorKind::ConvertedTooLarge);
}

#[tokio::test]
async fn xml_entities_roots_structure_and_bases_are_strict() {
    let valid = br#"<rss version="2.0"><channel><title>&amp; &#x20; &#9;</title><link>https://example.test/</link><description>x</description></channel></rss>"#;
    parse_fixture(valid, "application/rss+xml").await;

    let cases: &[(&[u8], FeedParseErrorKind)] = &[
        (
            br#"<rss version="2.0"><channel><title>&custom;</title></channel></rss>"#,
            FeedParseErrorKind::UnsupportedEntity,
        ),
        (
            br#"<rss version="2.0"><channel><title>&#0;</title></channel></rss>"#,
            FeedParseErrorKind::MalformedXml,
        ),
        (
            br#"<rss version="2.0"></rss><rss version="2.0"></rss>"#,
            FeedParseErrorKind::MalformedXml,
        ),
        (
            br#"<rss version="2.0"><channel>"#,
            FeedParseErrorKind::MalformedXml,
        ),
        (
            br#"<rss><channel></channel></rss>"#,
            FeedParseErrorKind::UnsupportedVersion,
        ),
        (
            br#"<rss version="2.0" xml:base="file:///tmp/"><channel></channel></rss>"#,
            FeedParseErrorKind::InvalidUrl,
        ),
        (
            br#"<rss version="2.0"><channel><link xml:base="https://ignored.test/">https://example.test/</link></channel></rss>"#,
            FeedParseErrorKind::InvalidUrl,
        ),
    ];
    for (body, kind) in cases {
        let error = parse(fetched(*body, Some("application/rss+xml")))
            .await
            .expect_err("unsafe XML rejects");
        assert_eq!(error.kind(), *kind);
    }
}

#[tokio::test]
async fn exact_feed_signatures_reject_unknown_or_non_feed_roots() {
    let cases: &[(&[u8], FeedParseErrorKind)] = &[
        (
            br#"<html><body>not a feed</body></html>"#,
            FeedParseErrorKind::MimeMismatch,
        ),
        (
            br#"<feed xmlns="https://wrong.example/atom"></feed>"#,
            FeedParseErrorKind::UnsupportedVersion,
        ),
        (
            br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"></rdf:RDF>"#,
            FeedParseErrorKind::UnsupportedVersion,
        ),
        (
            br#"<rss version="9.9"><channel></channel></rss>"#,
            FeedParseErrorKind::UnsupportedVersion,
        ),
    ];
    for (body, kind) in cases {
        let error = parse(fetched(*body, Some("application/xml")))
            .await
            .expect_err("signature rejects");
        assert_eq!(error.kind(), *kind);
    }
    let json = parse(fetched(b"{\"hello\":\"world\"}", Some("application/json")))
        .await
        .expect_err("non-feed JSON rejects");
    assert_eq!(json.kind(), FeedParseErrorKind::MimeMismatch);
}

#[tokio::test]
async fn xml_attribute_depth_and_value_budgets_are_typed() {
    let attributes = (0..257)
        .map(|index| format!(" a{index}=\"x\""))
        .collect::<String>();
    let too_many = format!("<rss version=\"2.0\"{attributes}><channel></channel></rss>");
    let error = parse(fetched(too_many, Some("application/rss+xml")))
        .await
        .expect_err("attribute count rejects");
    assert_eq!(error.kind(), FeedParseErrorKind::AttributeCountLimit);

    let value = format!(
        "<rss version=\"2.0\" x=\"{}\"><channel></channel></rss>",
        "x".repeat(64 * 1024 + 1)
    );
    let error = parse(fetched(value, Some("application/rss+xml")))
        .await
        .expect_err("attribute value rejects");
    assert_eq!(error.kind(), FeedParseErrorKind::AttributeValueLimit);

    let mut deep = "<rss version=\"2.0\"><channel>".to_owned();
    for _ in 0..127 {
        deep.push_str("<x>");
    }
    for _ in 0..127 {
        deep.push_str("</x>");
    }
    deep.push_str("</channel></rss>");
    let error = parse(fetched(deep, Some("application/rss+xml")))
        .await
        .expect_err("depth rejects");
    assert_eq!(error.kind(), FeedParseErrorKind::DepthLimit);
}

#[tokio::test]
async fn parser_sentinels_prevent_text_content_enclosure_and_entry_truncation() {
    let feed_with_description = |length: usize| {
        format!(
            "<rss version=\"2.0\"><channel><title>x</title><link>https://example.test/</link><item><guid>x</guid><description>{}</description></item></channel></rss>",
            "a".repeat(length)
        )
    };
    parse(fetched(
        feed_with_description(1024 * 1024),
        Some("application/rss+xml"),
    ))
    .await
    .expect("1 MiB parser text survives intact");
    let error = parse(fetched(
        feed_with_description(1024 * 1024 + 1),
        Some("application/rss+xml"),
    ))
    .await
    .expect_err("1 MiB + 1 domain text rejects after sentinel parse");
    assert_eq!(error.kind(), FeedParseErrorKind::ContentTooLong);

    let title = format!(
        "<rss version=\"2.0\"><channel><title>{}</title><link>https://example.test/</link></channel></rss>",
        "t".repeat(64 * 1024 + 1)
    );
    let error = parse(fetched(title, Some("application/rss+xml")))
        .await
        .expect_err("title cap rejects");
    assert_eq!(error.kind(), FeedParseErrorKind::TitleTooLong);

    let rss_enclosures = |count: usize, long: bool| {
        let enclosures = (0..count)
            .map(|index| {
                let path = if long { "x".repeat(4_070) } else { index.to_string() };
                format!("<enclosure url=\"https://cdn.example.test/{path}\" type=\"audio/mpeg\" length=\"1\"/>")
            })
            .collect::<String>();
        format!(
            "<rss version=\"2.0\"><channel><title>x</title><link>https://example.test/</link><item><guid>x</guid>{enclosures}</item></channel></rss>"
        )
    };
    assert_eq!(
        parse(fetched(
            rss_enclosures(64, false),
            Some("application/rss+xml")
        ))
        .await
        .expect("64 enclosures")
        .entries()[0]
            .enclosures()
            .len(),
        64
    );
    let error = parse(fetched(
        rss_enclosures(65, false),
        Some("application/rss+xml"),
    ))
    .await
    .expect_err("65 enclosures reject");
    assert_eq!(error.kind(), FeedParseErrorKind::TooManyEnclosures);
    let error = parse(fetched(
        rss_enclosures(64, true),
        Some("application/rss+xml"),
    ))
    .await
    .expect_err("canonical enclosure JSON budget rejects");
    assert_eq!(error.kind(), FeedParseErrorKind::EnclosureJsonTooLarge);

    let atom_blocks = |count: usize| {
        let content = (0..count)
            .map(|index| format!("<content type=\"text\">block-{index}</content>"))
            .collect::<String>();
        format!(
            "<feed xmlns=\"http://www.w3.org/2005/Atom\"><title>x</title><id>x</id><updated>2026-07-16T00:00:00Z</updated><entry><id>x</id><title>x</title><updated>2026-07-16T00:00:00Z</updated>{content}</entry></feed>"
        )
    };
    parse(fetched(atom_blocks(64), Some("application/atom+xml")))
        .await
        .expect("64 content blocks");
    let error = parse(fetched(atom_blocks(65), Some("application/atom+xml")))
        .await
        .expect_err("65 content blocks reject");
    assert_eq!(error.kind(), FeedParseErrorKind::TooManyContentBlocks);

    let rss_entries = |count: usize| {
        let items = (0..count)
            .map(|index| format!("<item><guid>{index}</guid><description>x</description></item>"))
            .collect::<String>();
        format!(
            "<rss version=\"2.0\"><channel><title>x</title><link>https://example.test/</link>{items}</channel></rss>"
        )
    };
    assert_eq!(
        parse(fetched(rss_entries(5_000), Some("application/rss+xml")))
            .await
            .expect("5,000 entries")
            .entries()
            .len(),
        5_000
    );
    let error = parse(fetched(rss_entries(5_001), Some("application/rss+xml")))
        .await
        .expect_err("5,001 entries reject");
    assert!(matches!(
        error.kind(),
        FeedParseErrorKind::BozoRejected | FeedParseErrorKind::TooManyEntries
    ));
}

#[tokio::test]
async fn json_n_plus_one_and_depth_prechecks_reject_before_parser_truncation() {
    let item = |content_len: usize| {
        serde_json::json!({
            "version": "https://jsonfeed.org/version/1.1",
            "title": "x",
            "items": [{"id":"x", "content_text":"x".repeat(content_len)}]
        })
        .to_string()
    };
    parse(fetched(item(1024 * 1024), Some("application/json")))
        .await
        .expect("1 MiB JSON content accepted");
    let error = parse(fetched(item(1024 * 1024 + 1), Some("application/json")))
        .await
        .expect_err("1 MiB + 1 JSON rejects");
    assert_eq!(error.kind(), FeedParseErrorKind::ContentTooLong);

    let attachments = (0..65)
        .map(|index| serde_json::json!({"url":format!("https://example.test/{index}"),"mime_type":"x/y"}))
        .collect::<Vec<_>>();
    let error = parse(fetched(
            serde_json::json!({"version":"https://jsonfeed.org/version/1.1","title":"x","items":[{"id":"x","attachments":attachments}]}).to_string(),
            Some("application/json"),
        ))
        .await
        .expect_err("65 JSON attachments reject");
    assert_eq!(error.kind(), FeedParseErrorKind::TooManyEnclosures);

    let items = (0..5_001)
        .map(|index| serde_json::json!({"id":index.to_string(),"content_text":"x"}))
        .collect::<Vec<_>>();
    let error = parse(fetched(
        serde_json::json!({"version":"https://jsonfeed.org/version/1.1","title":"x","items":items})
            .to_string(),
        Some("application/json"),
    ))
    .await
    .expect_err("5,001 JSON entries reject");
    assert_eq!(error.kind(), FeedParseErrorKind::TooManyEntries);

    let mut deep =
        String::from("{\"version\":\"https://jsonfeed.org/version/1.1\",\"items\":[],\"x\":");
    deep.push_str(&"[".repeat(129));
    deep.push('0');
    deep.push_str(&"]".repeat(129));
    deep.push('}');
    let error = parse(fetched(deep, Some("application/json")))
        .await
        .expect_err("JSON depth rejects");
    assert_eq!(error.kind(), FeedParseErrorKind::DepthLimit);
}

#[tokio::test]
async fn duplicate_identity_precedence_newer_and_ties_are_deterministic() {
    let parsed = parse_fixture(IDENTITY_EDGES, "application/rss+xml").await;
    assert_eq!(parsed.entries().len(), 4);
    assert_eq!(parsed.duplicate_count(), 2);
    let duplicate = parsed
        .entries()
        .iter()
        .find(|entry| entry.identity().identity() == "duplicate-guid")
        .expect("duplicate remains");
    assert_eq!(duplicate.title(), Some("Newer duplicate"));
    let tie = parsed
        .entries()
        .iter()
        .find(|entry| entry.identity().identity() == "tie-guid")
        .expect("tie remains");
    assert_eq!(tie.title(), Some("First tie"));
    assert!(
        parsed
            .entries()
            .iter()
            .any(|entry| entry.identity().kind() == IdentityKind::Url)
    );
    assert!(
        parsed
            .entries()
            .iter()
            .any(|entry| entry.identity().kind() == IdentityKind::Fingerprint)
    );
}

#[tokio::test]
async fn malicious_html_is_inert_and_image_metadata_stays_out_of_band() {
    let parsed = parse_fixture(MALICIOUS_HTML, "application/rss+xml").await;
    let content = parsed.entries()[0].content();
    let html = content.html();
    for forbidden in [
        "<script",
        "<style",
        "<form",
        "<iframe",
        "<svg",
        "<math",
        "<template",
        "onclick",
        "onerror",
        "src=",
        "srcset",
        "ping=",
        "style=",
        "class=",
        "id=",
        "data-",
        "javascript:",
        "publisher=secret",
    ] {
        assert!(
            !html.contains(forbidden),
            "forbidden sanitized fragment: {forbidden}"
        );
    }
    assert!(html.contains("href=\"https://feeds.example.test/good\""));
    assert!(html.contains("rel=\"noopener noreferrer nofollow\""));
    assert_no_fetch_capable_dom_attributes(html);
    assert_eq!(content.images().len(), 1);
    let image = &content.images()[0];
    assert_eq!(image.image_index(), 0);
    assert_eq!(
        image.source_url(),
        "https://img.example.test/safe.jpg?publisher=secret"
    );
    assert_eq!(image.alt(), Some("Safe image"));
    assert_eq!(image.width(), Some(640));
    assert_eq!(image.height(), Some(360));
}

#[test]
fn sanitizer_enforces_image_and_final_html_budgets() {
    let base = "https://example.test/";
    let accepted = format!(
        "<img src='/x' alt='{}' width='16384' height='1'>",
        "a".repeat(4096)
    );
    assert!(sanitize_entry_html(base, &accepted).is_ok());

    let alt = format!("<img src='/x' alt='{}'>", "a".repeat(4097));
    assert_eq!(
        sanitize_entry_html(base, &alt).expect_err("alt cap"),
        SanitizeError::ImageAltTooLong { bytes: 4097 }
    );
    for dimension in ["0", "16385", "not-a-number"] {
        let html = format!("<img src='/x' width='{dimension}'>");
        assert_eq!(
            sanitize_entry_html(base, &html).expect_err("dimension cap"),
            SanitizeError::ImageDimensionInvalid
        );
    }
    let images = (0..257)
        .map(|index| format!("<img src='/images/{index}.jpg' alt='{index}'>"))
        .collect::<String>();
    assert_eq!(
        sanitize_entry_html(base, &images).expect_err("image count"),
        SanitizeError::TooManyImages { count: 257 }
    );
    let metadata = (0..65)
        .map(|index| {
            format!(
                "<img src='https://img.example.test/{}/{}' alt='{index}'>",
                "x".repeat(4_000),
                index
            )
        })
        .collect::<String>();
    assert!(matches!(
        sanitize_entry_html(base, &metadata),
        Err(SanitizeError::ImageMetadataTooLarge { .. })
    ));
    let oversized = format!("<p>{}</p>", "x".repeat(1024 * 1024));
    assert!(matches!(
        sanitize_entry_html(base, &oversized),
        Err(SanitizeError::FinalHtmlTooLong { .. })
    ));
}

#[tokio::test]
async fn sanitizer_limit_errors_map_to_stable_feed_parse_categories() {
    let feed = |html: String| {
        format!(
            "<rss version=\"2.0\"><channel><title>x</title><link>https://example.test/</link><item><guid>x</guid><description><![CDATA[{html}]]></description></item></channel></rss>"
        )
    };
    let oversized = parse(fetched(
        feed("&".repeat(300_000)),
        Some("application/rss+xml"),
    ))
    .await
    .expect_err("sanitized expansion rejects");
    assert_eq!(
        oversized.kind(),
        FeedParseErrorKind::SanitizedContentTooLong
    );

    let alt = parse(fetched(
        feed(format!("<img src='/x' alt='{}'>", "a".repeat(4097))),
        Some("application/rss+xml"),
    ))
    .await
    .expect_err("alt rejects");
    assert_eq!(alt.kind(), FeedParseErrorKind::ImageAltTooLong);

    let dimension = parse(fetched(
        feed("<img src='/x' width='16385'>".to_owned()),
        Some("application/rss+xml"),
    ))
    .await
    .expect_err("dimension rejects");
    assert_eq!(dimension.kind(), FeedParseErrorKind::ImageDimensionInvalid);
}

#[tokio::test]
async fn semantic_hash_ignores_tracking_style_and_image_source_only_changes() {
    let feed = |html: &str| {
        format!(
            "<rss version=\"2.0\"><channel><title>x</title><link>https://example.test/</link><item><guid>x</guid><description><![CDATA[{html}]]></description></item></channel></rss>"
        )
    };
    let first = parse(fetched(
            feed("<p class='a' style='color:red' data-track='1'>same<img src='https://img.example.test/a.jpg'></p>"),
            Some("application/rss+xml"),
        ))
        .await
        .expect("first parses");
    let second = parse(fetched(
            feed("<p class='b' style='color:blue' data-track='2'>same<img src='https://img.example.test/b.jpg'></p>"),
            Some("application/rss+xml"),
        ))
        .await
        .expect("second parses");
    let changed = parse(fetched(
        feed("<p>different<img src='https://img.example.test/b.jpg'></p>"),
        Some("application/rss+xml"),
    ))
    .await
    .expect("changed parses");
    assert_eq!(
        first.entries()[0].content_hash(),
        second.entries()[0].content_hash()
    );
    assert_ne!(
        first.entries()[0].content_hash(),
        changed.entries()[0].content_hash()
    );
    assert_ne!(
        first.entries()[0].source_content_hash(),
        first.entries()[0].content_hash()
    );
}

#[tokio::test]
async fn parse_errors_redact_url_body_html_validator_and_parser_exception() {
    let url = FeedUrlPolicy::new(true)
        .normalize("https://feeds.example.test/source.xml?super_secret_query=yes")
        .expect("URL");
    let validator = OpaqueValidator::from_header(HeaderValue::from_static("validator-secret"))
        .expect("validator");
    let document = FetchedDocument::try_from(FetchOutcome::Document {
        url,
        document: b"<rss version=\"2.0\"><channel><title>body-secret<script>html-secret</script>"
            .to_vec(),
        content_type: Some("application/rss+xml".to_owned()),
        etag: Some(validator),
        last_modified: None,
    })
    .expect("document");
    let error = parse(document).await.expect_err("malformed feed rejects");
    let rendered = format!("{error:?} {error}");
    for secret in [
        "super_secret_query",
        "body-secret",
        "html-secret",
        "validator-secret",
        "channel",
        "script",
    ] {
        assert!(!rendered.contains(secret), "error leaked {secret}");
    }
}

struct AttributeSink {
    attributes: RefCell<Vec<(String, String)>>,
}

impl TokenSink for AttributeSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
        if let TagToken(tag) = token
            && tag.kind == StartTag
        {
            for attribute in tag.attrs {
                self.attributes
                    .borrow_mut()
                    .push((tag.name.to_string(), attribute.name.local.to_string()));
            }
        }
        TokenSinkResult::Continue
    }
}

fn assert_no_fetch_capable_dom_attributes(html: &str) {
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let tokenizer = Tokenizer::new(
        AttributeSink {
            attributes: RefCell::new(Vec::new()),
        },
        TokenizerOpts::default(),
    );
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    for (tag, attribute) in tokenizer.sink.attributes.into_inner() {
        let allowed_anchor = tag == "a" && attribute == "href";
        assert!(
            allowed_anchor
                || !matches!(
                    attribute.as_str(),
                    "src"
                        | "srcset"
                        | "poster"
                        | "background"
                        | "ping"
                        | "action"
                        | "formaction"
                        | "href"
                ),
            "fetch-capable {tag}[{attribute}] remained"
        );
    }
}

#[tokio::test]
async fn versioned_fixture_manifest_freezes_hashes_and_ordered_entry_identity() {
    let fixtures: &[(&str, &[u8], &str)] = &[
        ("rss_2_60_items.xml", RSS_60, "application/rss+xml"),
        ("atom_mixed.xml", ATOM, "application/atom+xml"),
        ("rss_1_rdf.xml", RDF, "application/rdf+xml"),
        ("json_feed.json", JSON_FEED, "application/feed+json"),
        ("rss_windows_1252.xml", WINDOWS_1252, "application/rss+xml"),
        (
            "rss_identity_edges.xml",
            IDENTITY_EDGES,
            "application/rss+xml",
        ),
        ("malicious_html.xml", MALICIOUS_HTML, "application/rss+xml"),
    ];
    let mut files = serde_json::Map::new();
    for (name, bytes, content_type) in fixtures {
        let parsed = parse_fixture(bytes, content_type).await;
        assert_eq!(
            parsed.source().source_document_hash(),
            blake3::hash(bytes).as_bytes(),
            "source document hash freezes exact transport-decoded bytes for {name}"
        );
        let decoded = if *name == "rss_windows_1252.xml" {
            encoding_rs::WINDOWS_1252
                .decode_without_bom_handling_and_without_replacement(bytes)
                .expect("strict fixture decode")
                .into_owned()
        } else {
            String::from_utf8(bytes.to_vec()).expect("UTF-8 fixture")
        };
        files.insert(
            (*name).to_owned(),
            serde_json::json!({
                "format": parsed.version().as_str(),
                "expected_entries": parsed.entries().len(),
                "raw_hash": hex(blake3::hash(bytes).as_bytes()),
                "decoded_hash": hex(blake3::hash(decoded.as_bytes()).as_bytes()),
                "entries": parsed.entries().iter().map(|entry| serde_json::json!({
                    "identity_kind": entry.identity().kind().as_database_str(),
                    "identity": entry.identity().identity(),
                    "index_hash": entry.identity().index_hash(),
                    "content_hash": hex(entry.content_hash()),
                })).collect::<Vec<_>>()
            }),
        );
    }
    let unsafe_decoded = String::from_utf8(UNSAFE_XML.to_vec()).expect("UTF-8 unsafe fixture");
    files.insert(
        "unsafe_xml.xml".to_owned(),
        serde_json::json!({
            "expected_error": "DoctypeForbidden",
            "raw_hash": hex(blake3::hash(UNSAFE_XML).as_bytes()),
            "decoded_hash": hex(blake3::hash(unsafe_decoded.as_bytes()).as_bytes()),
            "entries": []
        }),
    );
    let actual = serde_json::json!({"version":1,"files":files});
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/feed_manifest.json"))
            .expect("manifest is valid JSON");
    assert_eq!(actual, expected);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
