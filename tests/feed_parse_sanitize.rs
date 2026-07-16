use std::{cell::RefCell, mem::size_of};

use html5ever::{
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, StartTag, TagToken, Token, TokenSink, TokenSinkResult, Tokenizer,
        TokenizerOpts,
    },
};
use http::HeaderValue;
use raindrop::feeds::{
    FeedParseError, FeedParseErrorKind, FeedParser, FeedUrlPolicy, FetchOutcome, FetchedDocument,
    FetchedDocumentError, IdentityKind, OpaqueValidator, ParsedFeed, ParsedFeedVersion,
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
const MAX_PROJECTED_INHERITANCE_BYTES: usize = 32 * 1024 * 1024;

fn person_struct_bytes() -> usize {
    size_of::<feedparser_rs::Person>()
}

fn small_string_struct_bytes() -> usize {
    size_of::<feedparser_rs::types::SmallString>()
}

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
async fn sniff_accepts_legal_xml_prologs_and_qualified_feed_roots() {
    for body in [
        br#"<!-- legal leading comment --><?probe ok?><rss version="2.0"><channel><title>x</title></channel></rss>"#.as_slice(),
        br#"<?probe ok?><atom:feed xmlns:atom="http://www.w3.org/2005/Atom"><title>x</title><id>x</id><updated>2026-07-16T00:00:00Z</updated></atom:feed>"#.as_slice(),
    ] {
        parse(fetched(body, None))
            .await
            .expect("legal XML prolog and qualified feed root sniff as XML");
    }

    let non_feed = br#"<!-- legal leading comment --><html><body>x</body></html>"#;
    assert_eq!(
        parse(fetched(non_feed, None))
            .await
            .expect_err("tolerated MIME does not classify a non-feed root")
            .kind(),
        FeedParseErrorKind::UnsupportedContentType
    );
    assert_eq!(
        parse(fetched(non_feed, Some("application/xml")))
            .await
            .expect_err("direct XML classifies a non-feed root as a mismatch")
            .kind(),
        FeedParseErrorKind::MimeMismatch
    );
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
        (
            br#"&amp;<rss version="2.0"><channel><title>x</title></channel></rss>"#,
            FeedParseErrorKind::MalformedXml,
        ),
        (
            br#"<rss version="2.0"><channel><title>x</title></channel></rss>&amp;"#,
            FeedParseErrorKind::MalformedXml,
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
async fn cdata_is_confined_to_the_root_and_xml_10_legal_characters() {
    for body in [
        "<![CDATA[outside]]><rss version=\"2.0\"><channel><title>x</title></channel></rss>"
            .to_owned(),
        "<rss version=\"2.0\"><channel><title><![CDATA[illegal\u{000b}character]]></title></channel></rss>"
            .to_owned(),
    ] {
        let error = parse(fetched(body, Some("application/rss+xml")))
            .await
            .expect_err("invalid CDATA rejects");
        assert_eq!(error.kind(), FeedParseErrorKind::MalformedXml);
    }
}

#[tokio::test]
async fn extension_and_empty_entries_cannot_shift_xml_sidecar_ordinals() {
    let extension = br#"
        <feed xmlns="http://www.w3.org/2005/Atom" xmlns:ext="urn:example:extension">
          <title>x</title><id>x</id><updated>2026-07-16T00:00:00Z</updated>
          <ext:entry xml:base="https://attacker.example/"><ext:link href="poison"/></ext:entry>
          <entry xml:base="https://good.example/posts/">
            <id>real</id><title>real</title><updated>2026-07-16T00:00:00Z</updated>
            <link rel="alternate" href="one"/>
          </entry>
        </feed>
    "#;
    let parsed = parse(fetched(extension, Some("application/atom+xml")))
        .await
        .expect("extension local names do not enter the sidecar");
    assert_eq!(parsed.entries().len(), 1);
    assert_eq!(
        parsed.entries()[0].canonical_url(),
        Some("https://good.example/posts/one")
    );

    let empty = br#"
        <feed xmlns="http://www.w3.org/2005/Atom">
          <title>x</title><id>x</id><updated>2026-07-16T00:00:00Z</updated>
          <entry xml:base="https://attacker.example/"/>
          <entry xml:base="https://good.example/posts/">
            <id>real</id><title>real</title><updated>2026-07-16T00:00:00Z</updated>
            <content type="html" xml:base="../assets/">&lt;img src="image.jpg" alt="real"&gt;</content>
            <link rel="enclosure" href="audio.mp3" type="audio/mpeg"/>
          </entry>
        </feed>
    "#;
    let parsed = parse(fetched(empty, Some("application/atom+xml")))
        .await
        .expect("self-closing empty entries do not enter the sidecar");
    let entry = &parsed.entries()[0];
    assert_eq!(
        entry.content().images()[0].source_url(),
        "https://good.example/assets/image.jpg"
    );
    assert_eq!(
        entry.enclosures()[0].url(),
        "https://good.example/posts/audio.mp3"
    );
}

#[tokio::test]
async fn xml_sidecar_and_parser_collection_mismatches_fail_closed() {
    for body in [
        br#"<feed xmlns="http://www.w3.org/2005/Atom"><title>x</title><id>x</id><updated>2026-07-16T00:00:00Z</updated><link/><entry><id>x</id><updated>2026-07-16T00:00:00Z</updated></entry></feed>"#.as_slice(),
        br#"<rss version="2.0"><channel><title>x</title><item><guid isPermaLink="false">x</guid><enclosure/></item></channel></rss>"#.as_slice(),
    ] {
        let error = parse(fetched(body, Some("application/xml")))
            .await
            .expect_err("sidecar and parser collection mismatch rejects");
        assert_eq!(error.kind(), FeedParseErrorKind::ParserFailure);
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

    let invalid_rdf = br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlnsfoo="http://purl.org/rss/1.0/"></rdf:RDF>"#;
    let error = parse(fetched(invalid_rdf, Some("application/rdf+xml")))
        .await
        .expect_err("an xmlns-prefixed ordinary attribute is not a namespace declaration");
    assert_eq!(error.kind(), FeedParseErrorKind::UnsupportedVersion);

    for valid_rdf in [
        br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns="http://purl.org/rss/1.0/"><channel rdf:about="https://example.test/"><title>x</title><link>https://example.test/</link><description>x</description></channel></rdf:RDF>"#.as_slice(),
        br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:rss="http://purl.org/rss/1.0/"><rss:channel rdf:about="https://example.test/"><rss:title>x</rss:title><rss:link>https://example.test/</rss:link><rss:description>x</rss:description></rss:channel></rdf:RDF>"#.as_slice(),
    ] {
        let parsed = parse(fetched(valid_rdf, Some("application/rdf+xml")))
            .await
            .expect("default and prefixed RSS 1.0 namespace declarations are valid");
        assert_eq!(parsed.version(), ParsedFeedVersion::Rss10);
    }
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
async fn summary_limit_is_enforced_even_when_content_wins_selection() {
    let feed = |summary_len: usize| {
        format!(
            "<feed xmlns=\"http://www.w3.org/2005/Atom\"><title>x</title><id>x</id><updated>2026-07-16T00:00:00Z</updated><entry><id>x</id><title>x</title><updated>2026-07-16T00:00:00Z</updated><summary>{}</summary><content type=\"text\">small</content></entry></feed>",
            "s".repeat(summary_len)
        )
    };
    parse(fetched(feed(1024 * 1024), Some("application/atom+xml")))
        .await
        .expect("exactly 1 MiB summary is accepted when content wins");
    let error = parse(fetched(feed(1024 * 1024 + 1), Some("application/atom+xml")))
        .await
        .expect_err("1 MiB + 1 summary rejects even when content wins");
    assert_eq!(error.kind(), FeedParseErrorKind::ContentTooLong);
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

    let nested = |arrays: usize| {
        let mut json =
            String::from("{\"version\":\"https://jsonfeed.org/version/1.1\",\"items\":[],\"x\":");
        json.push_str(&"[".repeat(arrays));
        json.push('0');
        json.push_str(&"]".repeat(arrays));
        json.push('}');
        json
    };
    parse(fetched(nested(127), Some("application/json")))
        .await
        .expect("exactly 128 nested JSON containers are accepted");

    let mut deep =
        String::from("{\"version\":\"https://jsonfeed.org/version/1.1\",\"items\":[],\"x\":");
    deep.push_str(&"[".repeat(128));
    deep.push('0');
    deep.push_str(&"]".repeat(128));
    deep.push('}');
    let error = parse(fetched(deep, Some("application/json")))
        .await
        .expect_err("129 nested JSON containers reject");
    assert_eq!(error.kind(), FeedParseErrorKind::DepthLimit);
}

#[tokio::test]
async fn json_feed_version_and_attachment_types_are_strict() {
    for value in [
        serde_json::json!({"title":"x","items":[]}),
        serde_json::json!({"version":null,"title":"x","items":[]}),
        serde_json::json!({"version":"https://jsonfeed.org/version/9","title":"x","items":[]}),
    ] {
        let error = parse(fetched(value.to_string(), Some("application/json")))
            .await
            .expect_err("missing or invalid JSON Feed version rejects");
        assert_eq!(error.kind(), FeedParseErrorKind::UnsupportedVersion);
    }

    for attachment in [
        serde_json::json!({"mime_type":"audio/mpeg"}),
        serde_json::json!({"url":null,"mime_type":"audio/mpeg"}),
        serde_json::json!({"url":7,"mime_type":"audio/mpeg"}),
        serde_json::json!({"url":"https://example.test/a"}),
        serde_json::json!({"url":"https://example.test/a","mime_type":null}),
        serde_json::json!({"url":"https://example.test/a","mime_type":1}),
        serde_json::json!({"url":"https://example.test/a","title":false}),
        serde_json::json!({"url":"https://example.test/a","size_in_bytes":-1}),
        serde_json::json!({"url":"https://example.test/a","size_in_bytes":1.5}),
        serde_json::json!({"url":"https://example.test/a","duration_in_seconds":"1"}),
    ] {
        let value = serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "items":[{"id":"x","content_text":"x","attachments":[attachment]}]
        });
        let error = parse(fetched(value.to_string(), Some("application/json")))
            .await
            .expect_err("wrong attachment field type rejects");
        assert_eq!(error.kind(), FeedParseErrorKind::MalformedJson);
    }

    let legacy_author = serde_json::json!({
        "version":"https://jsonfeed.org/version/1.1",
        "title":"x",
        "author":{"name":7},
        "items":[]
    });
    assert_eq!(
        parse(fetched(legacy_author.to_string(), Some("application/json")))
            .await
            .expect_err("legacy singular author fields are strict")
            .kind(),
        FeedParseErrorKind::MalformedJson
    );
}

#[tokio::test]
async fn json_feed_collection_and_string_boundaries_are_independent() {
    let authors = |count: usize| {
        (0..count)
            .map(|index| serde_json::json!({"name":format!("author-{index}")}))
            .collect::<Vec<_>>()
    };
    let hubs = |count: usize| {
        (0..count)
            .map(|index| {
                serde_json::json!({
                    "type":"WebSub",
                    "url":format!("https://hub.example.test/{index}")
                })
            })
            .collect::<Vec<_>>()
    };
    let tags = |count: usize| {
        (0..count)
            .map(|index| serde_json::Value::String(format!("tag-{index}")))
            .collect::<Vec<_>>()
    };

    for value in [
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "authors":authors(256),
            "items":[{"id":"x","content_text":"x"}]
        }),
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "hubs":hubs(256),
            "items":[]
        }),
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "items":[{"id":"x","content_text":"x","authors":authors(256),"tags":tags(256)}]
        }),
    ] {
        parse(fetched(value.to_string(), Some("application/json")))
            .await
            .expect("exact collection limits are accepted independently");
    }

    for value in [
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "authors":authors(257),
            "items":[]
        }),
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "hubs":hubs(257),
            "items":[]
        }),
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "items":[{"id":"x","content_text":"x","authors":authors(257)}]
        }),
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "items":[{"id":"x","content_text":"x","tags":tags(257)}]
        }),
    ] {
        assert_eq!(
            parse(fetched(value.to_string(), Some("application/json")))
                .await
                .expect_err("N + 1 JSON collection rejects before parser truncation")
                .kind(),
            FeedParseErrorKind::ParserFailure
        );
    }

    for value in [
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "authors":null,
            "items":[]
        }),
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "hubs":{},
            "items":[]
        }),
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "items":[{"id":"x","content_html":7}]
        }),
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "items":[{"id":"x","content_text":"x","authors":"author"}]
        }),
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "title":"x",
            "items":[{"id":"x","content_text":"x","tags":["ok",9]}]
        }),
    ] {
        assert_eq!(
            parse(fetched(value.to_string(), Some("application/json")))
                .await
                .expect_err("wrong JSON collection or string type rejects")
                .kind(),
            FeedParseErrorKind::MalformedJson
        );
    }
}

#[tokio::test]
async fn json_author_inheritance_budget_is_exact() {
    const ITEMS_AT_LIMIT: usize = 4_096;
    let structural = 2 * person_struct_bytes() + small_string_struct_bytes();
    let payload = MAX_PROJECTED_INHERITANCE_BYTES / ITEMS_AT_LIMIT - structural;
    let name_len = payload % 2;
    let avatar_len = (payload - 3 * name_len) / 2;
    let per_item = structural + 3 * name_len + 2 * avatar_len;
    assert_eq!(per_item * ITEMS_AT_LIMIT, MAX_PROJECTED_INHERITANCE_BYTES);

    let document = |item_count: usize| {
        let items = (0..item_count)
            .map(|index| serde_json::json!({"id":index.to_string(),"content_text":"x"}))
            .collect::<Vec<_>>();
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "authors":[{"name":"n".repeat(name_len),"avatar":"a".repeat(avatar_len)}],
            "items":items
        })
        .to_string()
    };

    let parsed = parse(fetched(
        document(ITEMS_AT_LIMIT),
        Some("application/feed+json"),
    ))
    .await
    .expect("exactly 32 MiB of projected JSON author inheritance is accepted");
    assert_eq!(parsed.entries().len(), ITEMS_AT_LIMIT);

    let expected = per_item * (ITEMS_AT_LIMIT + 1);
    let error = parse(fetched(
        document(ITEMS_AT_LIMIT + 1),
        Some("application/feed+json"),
    ))
    .await
    .expect_err("JSON author inheritance above 32 MiB rejects");
    assert_eq!(
        error.kind(),
        FeedParseErrorKind::ProjectedInheritanceTooLarge
    );
    assert_eq!(error.byte_length(), Some(expected));
}

#[tokio::test]
async fn json_language_inheritance_budget_is_exact() {
    const ITEMS_AT_LIMIT: usize = 4_096;
    let language_len =
        MAX_PROJECTED_INHERITANCE_BYTES / ITEMS_AT_LIMIT / 2 - small_string_struct_bytes();
    let per_item = 2 * (small_string_struct_bytes() + language_len);
    assert_eq!(per_item * ITEMS_AT_LIMIT, MAX_PROJECTED_INHERITANCE_BYTES);

    let document = |item_count: usize| {
        let items = (0..item_count)
            .map(|index| serde_json::json!({"id":index.to_string(),"content_text":"x"}))
            .collect::<Vec<_>>();
        serde_json::json!({
            "version":"https://jsonfeed.org/version/1.1",
            "language":"l".repeat(language_len),
            "items":items
        })
        .to_string()
    };

    let parsed = parse(fetched(
        document(ITEMS_AT_LIMIT),
        Some("application/feed+json"),
    ))
    .await
    .expect("exactly 32 MiB of projected JSON language inheritance is accepted");
    assert_eq!(parsed.entries().len(), ITEMS_AT_LIMIT);

    let expected = per_item * (ITEMS_AT_LIMIT + 1);
    let error = parse(fetched(
        document(ITEMS_AT_LIMIT + 1),
        Some("application/feed+json"),
    ))
    .await
    .expect_err("JSON language inheritance above 32 MiB rejects");
    assert_eq!(
        error.kind(),
        FeedParseErrorKind::ProjectedInheritanceTooLarge
    );
    assert_eq!(error.byte_length(), Some(expected));
}

#[tokio::test]
async fn atom_author_inheritance_budget_is_exact() {
    let (entries_at_limit, name_len, uri_len) = (2_000..5_000)
        .find_map(|entry_count| {
            let structural = (entry_count + 1) * person_struct_bytes()
                + entry_count * small_string_struct_bytes();
            let payload_budget = MAX_PROJECTED_INHERITANCE_BYTES.checked_sub(structural)?;
            let name_coefficient = 2 * entry_count + 1;
            let uri_coefficient = entry_count + 1;
            (0..=64 * 1024).find_map(|name_len| {
                let name_bytes = name_coefficient * name_len;
                let remainder = payload_budget.checked_sub(name_bytes)?;
                if remainder % uri_coefficient == 0 {
                    let uri_len = remainder / uri_coefficient;
                    (uri_len <= 1024 * 1024).then_some((entry_count, name_len, uri_len))
                } else {
                    None
                }
            })
        })
        .expect("an exact bounded Atom author payload solution exists");
    let vector_clone = person_struct_bytes() + name_len + uri_len;
    let flat_clone = small_string_struct_bytes() + name_len;
    let projected = vector_clone + entries_at_limit * (vector_clone + flat_clone);
    assert_eq!(projected, MAX_PROJECTED_INHERITANCE_BYTES);

    let document = |entry_count: usize| {
        let mut xml = format!(
            "<feed xmlns='http://www.w3.org/2005/Atom'><title>x</title><id>x</id><updated>2026-07-16T00:00:00Z</updated><author><name>{}</name><uri>{}</uri></author>",
            "n".repeat(name_len),
            "u".repeat(uri_len)
        );
        for index in 0..entry_count {
            xml.push_str(&format!(
                "<entry><id>{index}</id><title>x</title><updated>2026-07-16T00:00:00Z</updated></entry>"
            ));
        }
        xml.push_str("</feed>");
        xml
    };

    let parsed = parse(fetched(
        document(entries_at_limit),
        Some("application/atom+xml"),
    ))
    .await
    .expect("exactly 32 MiB of projected Atom author inheritance is accepted");
    assert_eq!(parsed.entries().len(), entries_at_limit);

    let expected = projected + vector_clone + flat_clone;
    let error = parse(fetched(
        document(entries_at_limit + 1),
        Some("application/atom+xml"),
    ))
    .await
    .expect_err("Atom author inheritance above 32 MiB rejects");
    assert_eq!(
        error.kind(),
        FeedParseErrorKind::ProjectedInheritanceTooLarge
    );
    assert_eq!(error.byte_length(), Some(expected));
}

#[tokio::test]
async fn atom_language_inheritance_budget_is_exact() {
    const ENTRIES_AT_LIMIT: usize = 4_096;
    let language_len =
        MAX_PROJECTED_INHERITANCE_BYTES / ENTRIES_AT_LIMIT - small_string_struct_bytes();
    let per_entry = small_string_struct_bytes() + language_len;
    assert_eq!(
        per_entry * ENTRIES_AT_LIMIT,
        MAX_PROJECTED_INHERITANCE_BYTES
    );

    let document = |entry_count: usize| {
        let mut xml = format!(
            "<feed xmlns='http://www.w3.org/2005/Atom' xml:lang='{}'><title xml:lang=''>x</title><id>x</id><updated>2026-07-16T00:00:00Z</updated>",
            "l".repeat(language_len)
        );
        for index in 0..entry_count {
            xml.push_str(&format!(
                "<entry><id>{index}</id><title>x</title><updated>2026-07-16T00:00:00Z</updated></entry>"
            ));
        }
        xml.push_str("</feed>");
        xml
    };

    let parsed = parse(fetched(
        document(ENTRIES_AT_LIMIT),
        Some("application/atom+xml"),
    ))
    .await
    .expect("exactly 32 MiB of projected Atom language inheritance is accepted");
    assert_eq!(parsed.entries().len(), ENTRIES_AT_LIMIT);

    let expected = per_entry * (ENTRIES_AT_LIMIT + 1);
    let error = parse(fetched(
        document(ENTRIES_AT_LIMIT + 1),
        Some("application/atom+xml"),
    ))
    .await
    .expect_err("Atom language inheritance above 32 MiB rejects");
    assert_eq!(
        error.kind(),
        FeedParseErrorKind::ProjectedInheritanceTooLarge
    );
    assert_eq!(error.byte_length(), Some(expected));
}

#[tokio::test]
async fn empty_feed_authors_cannot_bypass_structural_inheritance_budget() {
    let authors = (0..256).map(|_| serde_json::json!({})).collect::<Vec<_>>();
    let items = (0..5_000)
        .map(|index| serde_json::json!({"id":index.to_string(),"content_text":"x"}))
        .collect::<Vec<_>>();
    let projected = 5_000 * 257 * person_struct_bytes();
    assert!(projected > MAX_PROJECTED_INHERITANCE_BYTES);
    let document = serde_json::json!({
        "version":"https://jsonfeed.org/version/1.1",
        "authors":authors,
        "items":items
    });
    let error = parse(fetched(document.to_string(), Some("application/feed+json")))
        .await
        .expect_err("structural Person clones count even when every payload is empty");
    assert_eq!(
        error.kind(),
        FeedParseErrorKind::ProjectedInheritanceTooLarge
    );
    assert_eq!(error.byte_length(), Some(projected));
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

#[tokio::test]
async fn sanitizer_removes_namespaced_and_legacy_fetch_attributes() {
    let attributes = [
        "src",
        "srcset",
        "href",
        "xlink:href",
        "poster",
        "background",
        "action",
        "formaction",
        "cite",
        "ping",
        "usemap",
        "profile",
        "manifest",
        "archive",
        "codebase",
        "data",
        "icon",
        "longdesc",
        "lowsrc",
        "dynsrc",
    ];
    let poisoned = attributes
        .iter()
        .map(|attribute| format!(" {attribute}='https://fetch.example.test/{attribute}'"))
        .collect::<String>();
    let html = format!(
        "<a href='https://safe.example.test/' xlink:href='https://fetch.example.test/xlink' ping='https://fetch.example.test/ping'>safe</a><div{poisoned}>x</div><img src='https://safe.example.test/image.jpg' srcset='https://fetch.example.test/2x 2x' longdesc='https://fetch.example.test/longdesc' lowsrc='https://fetch.example.test/lowsrc' dynsrc='https://fetch.example.test/dynsrc' usemap='#map'><blockquote cite='https://fetch.example.test/cite'>q</blockquote><form action='https://fetch.example.test/action'><button formaction='https://fetch.example.test/formaction'>x</button></form><video poster='https://fetch.example.test/poster'></video><object data='https://fetch.example.test/data' codebase='https://fetch.example.test/codebase' archive='https://fetch.example.test/archive'></object><svg><image href='https://fetch.example.test/svg' xlink:href='https://fetch.example.test/xlink-svg'/></svg><math href='https://fetch.example.test/math'></math>"
    );
    let feed = format!(
        "<rss version=\"2.0\"><channel><title>x</title><link>https://example.test/</link><item><guid>x</guid><description><![CDATA[{html}]]></description></item></channel></rss>"
    );
    let parsed = parse(fetched(feed, Some("application/rss+xml")))
        .await
        .expect("legacy fetch attributes sanitize");
    let sanitized = parsed.entries()[0].content().html();
    assert!(sanitized.contains("href=\"https://safe.example.test/\""));
    assert_no_fetch_capable_dom_attributes(sanitized);
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
                let local = attribute.name.local.to_string();
                let name = attribute
                    .name
                    .prefix
                    .as_ref()
                    .map_or_else(|| local.clone(), |prefix| format!("{prefix}:{local}"));
                self.attributes
                    .borrow_mut()
                    .push((tag.name.to_string(), name));
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
        let local = attribute.rsplit(':').next().unwrap_or(&attribute);
        assert!(
            allowed_anchor
                || !matches!(
                    local,
                    "src"
                        | "srcset"
                        | "poster"
                        | "background"
                        | "ping"
                        | "action"
                        | "formaction"
                        | "cite"
                        | "usemap"
                        | "profile"
                        | "manifest"
                        | "archive"
                        | "codebase"
                        | "data"
                        | "icon"
                        | "longdesc"
                        | "lowsrc"
                        | "dynsrc"
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
