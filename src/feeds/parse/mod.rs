mod encoding;
mod finalize;
mod json;
mod map;
mod mime;
mod types;
mod xml;

use std::sync::{Arc, OnceLock};

use tokio::sync::{Semaphore, TryAcquireError};

pub use types::{
    FeedParseError, FeedParseErrorKind, FetchedDocument, FetchedDocumentError, ParsedEnclosure,
    ParsedEntry, ParsedFeed, ParsedFeedVersion, ParsedSource,
};

use self::{
    finalize::finalize,
    map::{map_feed, parser_limits},
    mime::BodyFormat,
    types::{FeedParseErrorKind as ErrorKind, parsed_source},
};

static PARSER_CAPACITY: OnceLock<Arc<Semaphore>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default)]
pub struct FeedParser;

impl FeedParser {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub async fn parse(&self, document: FetchedDocument) -> Result<ParsedFeed, FeedParseError> {
        self.parse_with_hooks(document, WorkerHooks::default())
            .await
    }

    async fn parse_with_hooks(
        &self,
        document: FetchedDocument,
        hooks: WorkerHooks,
    ) -> Result<ParsedFeed, FeedParseError> {
        let semaphore = PARSER_CAPACITY
            .get_or_init(|| Arc::new(Semaphore::new(2)))
            .clone();
        let permit = match semaphore.try_acquire_owned() {
            Ok(permit) => permit,
            Err(TryAcquireError::NoPermits) => {
                return Err(FeedParseError::new(ErrorKind::ParserBusy));
            }
            Err(TryAcquireError::Closed) => {
                return Err(FeedParseError::new(ErrorKind::SemaphoreClosed));
            }
        };
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            (hooks.on_start)();
            let result = parse_document(document);
            (hooks.on_finish)();
            result
        })
        .await
        .map_err(|_| FeedParseError::new(ErrorKind::WorkerPanicked))?
    }
}

fn parse_document(document: FetchedDocument) -> Result<ParsedFeed, FeedParseError> {
    let final_url = document.url.complete().to_owned();
    let mime = mime::classify(document.content_type.as_deref())?;
    let decoded = encoding::decode(&document.body, mime.charset.as_deref())?;
    let sniffed = mime::sniff(&decoded.utf8);
    let format = match (mime.expected, sniffed) {
        (Some(expected), Some(actual)) if expected == actual => actual,
        (Some(_), _) => return Err(FeedParseError::new(ErrorKind::MimeMismatch)),
        (None, Some(actual)) => actual,
        (None, None) => return Err(FeedParseError::new(ErrorKind::UnsupportedContentType)),
    };

    let preflight = match format {
        BodyFormat::Xml => Preflight::Xml(xml::preflight(&decoded.utf8, &final_url)?),
        BodyFormat::Json => Preflight::Json(json::preflight(&decoded.utf8)?),
    };
    let parser_bytes = match &preflight {
        Preflight::Xml(preflight) => preflight.parser_bytes.as_slice(),
        Preflight::Json(preflight) => preflight.parser_bytes.as_slice(),
    };
    let parsed = feedparser_rs::parse_with_limits(parser_bytes, parser_limits())
        .map_err(classify_parser_error)?;
    let xml_preflight = match &preflight {
        Preflight::Xml(preflight) => Some(preflight),
        Preflight::Json(_) => None,
    };
    let mapped = map_feed(parsed, &final_url, xml_preflight)?;
    let version = mapped.version;
    let title = mapped.title.clone();
    let canonical_url = mapped.canonical_url.clone();
    let (entries, duplicate_count) = finalize(mapped.entries)?;
    let source = parsed_source(
        document,
        decoded.original_encoding,
        decoded.source_document_hash,
    );
    Ok(ParsedFeed {
        source,
        version,
        title,
        canonical_url,
        entries,
        duplicate_count,
    })
}

enum Preflight {
    Xml(xml::PreflightedXml),
    Json(json::PreflightedJson),
}

fn classify_parser_error(error: feedparser_rs::FeedError) -> FeedParseError {
    let category = error.to_string();
    if category.contains("Text field") || category.contains("text length") {
        FeedParseError::new(ErrorKind::ContentTooLong)
    } else {
        FeedParseError::new(ErrorKind::ParserFailure)
    }
}

struct WorkerHooks {
    on_start: Box<dyn FnOnce() + Send + 'static>,
    on_finish: Box<dyn FnOnce() + Send + 'static>,
}

impl Default for WorkerHooks {
    fn default() -> Self {
        Self {
            on_start: Box::new(|| {}),
            on_finish: Box::new(|| {}),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Condvar, Mutex as StdMutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use super::*;
    use crate::feeds::{FeedUrlPolicy, FetchOutcome};

    static PARSER_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn document() -> FetchedDocument {
        let url = FeedUrlPolicy::new(true)
            .normalize("https://example.test/feed.xml")
            .expect("valid URL");
        FetchedDocument::try_from(FetchOutcome::Document {
            url,
            document: br#"<rss version="2.0"><channel><title>x</title></channel></rss>"#.to_vec(),
            content_type: Some("application/rss+xml".to_owned()),
            etag: None,
            last_modified: None,
        })
        .expect("document")
    }

    fn retained_body_document() -> FetchedDocument {
        let mut document = document();
        document.body = vec![b'x'; 1024 * 1024];
        document
    }

    struct DropProbe(Arc<AtomicUsize>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn aborted_awaiters_do_not_release_blocking_permits_early() {
        let _serial = PARSER_TEST_LOCK.lock().await;
        let gate = Arc::new((StdMutex::new(false), Condvar::new()));
        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let (finished_tx, mut finished_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut handles = Vec::new();
        for _ in 0..2 {
            let gate = gate.clone();
            let started_tx = started_tx.clone();
            let finished_tx = finished_tx.clone();
            handles.push(tokio::spawn(async move {
                FeedParser
                    .parse_with_hooks(
                        document(),
                        WorkerHooks {
                            on_start: Box::new(move || {
                                started_tx.send(()).expect("receiver remains");
                                let (lock, condition) = &*gate;
                                let mut released = lock.lock().expect("gate lock");
                                while !*released {
                                    released = condition.wait(released).expect("gate wait");
                                }
                            }),
                            on_finish: Box::new(move || {
                                finished_tx.send(()).expect("receiver remains");
                            }),
                        },
                    )
                    .await
            }));
        }
        started_rx.recv().await.expect("first worker started");
        started_rx.recv().await.expect("second worker started");
        for handle in &handles {
            handle.abort();
        }
        const BUSY_CALLERS: usize = 64;
        let start = Arc::new(tokio::sync::Barrier::new(BUSY_CALLERS + 1));
        let dropped = Arc::new(AtomicUsize::new(0));
        let mut busy_handles = Vec::with_capacity(BUSY_CALLERS);
        for _ in 0..BUSY_CALLERS {
            let start = start.clone();
            let dropped = dropped.clone();
            busy_handles.push(tokio::spawn(async move {
                let _drop_probe = DropProbe(dropped);
                let document = retained_body_document();
                start.wait().await;
                FeedParser.parse(document).await
            }));
        }
        start.wait().await;
        tokio::time::timeout(Duration::from_secs(1), async {
            for handle in busy_handles {
                let error = handle
                    .await
                    .expect("busy caller task completes")
                    .expect_err("saturation fails fast without a retained-body queue");
                assert_eq!(error.kind(), FeedParseErrorKind::ParserBusy);
            }
        })
        .await
        .expect("all simultaneous busy callers return promptly");
        assert_eq!(dropped.load(Ordering::SeqCst), BUSY_CALLERS);

        let (lock, condition) = &*gate;
        *lock.lock().expect("gate lock") = true;
        condition.notify_all();
        finished_rx.recv().await.expect("first worker exited");
        finished_rx.recv().await.expect("second worker exited");
    }

    #[tokio::test]
    async fn worker_panics_are_typed_without_exposing_the_panic_payload() {
        let _serial = PARSER_TEST_LOCK.lock().await;
        let error = FeedParser
            .parse_with_hooks(
                document(),
                WorkerHooks {
                    on_start: Box::new(|| panic!("publisher-secret-panic")),
                    on_finish: Box::new(|| {}),
                },
            )
            .await
            .expect_err("worker panic rejects");
        assert_eq!(error.kind(), FeedParseErrorKind::WorkerPanicked);
        assert!(!format!("{error:?} {error}").contains("publisher-secret-panic"));
    }
}
