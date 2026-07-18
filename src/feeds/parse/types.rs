use std::{error::Error, fmt};

use time::OffsetDateTime;

use crate::content::SanitizedContent;

use super::super::{EntryIdentity, FetchOutcome, NormalizedFeedUrl, OpaqueValidator};

pub(crate) const MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const MAX_ENTRIES: usize = 5_000;
pub(crate) const MAX_DEPTH: usize = 128;
pub(crate) const MAX_EVENTS: usize = 1_000_000;
pub(crate) const MAX_ATTRIBUTES: usize = 256;
pub(crate) const MAX_ATTRIBUTE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_TITLE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_CONTENT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_TOTAL_TEXT_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_CONTENT_BLOCKS: usize = 64;
pub(crate) const MAX_ENCLOSURES: usize = 64;
pub(crate) const MAX_ENCLOSURE_JSON_BYTES: usize = 256 * 1024;
pub(crate) const MAX_PROJECTED_INHERITANCE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, Default)]
pub(crate) struct ProjectedInheritance {
    bytes: usize,
}

impl ProjectedInheritance {
    pub(crate) fn add(&mut self, bytes: usize) -> Result<(), FeedParseError> {
        self.bytes = self.bytes.checked_add(bytes).ok_or_else(|| {
            FeedParseError::new(FeedParseErrorKind::ProjectedInheritanceTooLarge)
                .with_bytes(usize::MAX)
        })?;
        Ok(())
    }

    pub(crate) fn add_product(&mut self, count: usize, bytes: usize) -> Result<(), FeedParseError> {
        let bytes = count.checked_mul(bytes).ok_or_else(|| {
            FeedParseError::new(FeedParseErrorKind::ProjectedInheritanceTooLarge)
                .with_bytes(usize::MAX)
        })?;
        self.add(bytes)
    }

    #[must_use]
    pub(crate) const fn bytes(self) -> usize {
        self.bytes
    }
}

pub(crate) fn validate_projected_inheritance(bytes: usize) -> Result<(), FeedParseError> {
    if bytes > MAX_PROJECTED_INHERITANCE_BYTES {
        Err(FeedParseError::new(FeedParseErrorKind::ProjectedInheritanceTooLarge).with_bytes(bytes))
    } else {
        Ok(())
    }
}

pub struct FetchedDocument {
    pub(crate) url: NormalizedFeedUrl,
    pub(crate) body: Vec<u8>,
    pub(crate) content_type: Option<String>,
    pub(crate) etag: Option<OpaqueValidator>,
    pub(crate) last_modified: Option<OpaqueValidator>,
}

impl FetchedDocument {
    #[must_use]
    pub fn body_len(&self) -> usize {
        self.body.len()
    }
}

impl TryFrom<FetchOutcome> for FetchedDocument {
    type Error = FetchedDocumentError;

    fn try_from(outcome: FetchOutcome) -> Result<Self, Self::Error> {
        match outcome {
            FetchOutcome::Document {
                url,
                document,
                content_type,
                etag,
                last_modified,
            } => {
                if document.len() > MAX_DOCUMENT_BYTES {
                    return Err(FetchedDocumentError::TooLarge {
                        bytes: document.len(),
                    });
                }
                Ok(Self {
                    url,
                    body: document,
                    content_type,
                    etag,
                    last_modified,
                })
            }
            FetchOutcome::NotModified { .. } => Err(FetchedDocumentError::NotDocument),
        }
    }
}

impl fmt::Debug for FetchedDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FetchedDocument")
            .field("url", &self.url)
            .field("body_bytes", &self.body.len())
            .field(
                "content_type",
                &self.content_type.as_ref().map(|_| "[PRESENT]"),
            )
            .field("etag", &self.etag.as_ref().map(|_| "[REDACTED]"))
            .field(
                "last_modified",
                &self.last_modified.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchedDocumentError {
    NotDocument,
    TooLarge { bytes: usize },
}

impl fmt::Display for FetchedDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDocument => formatter.write_str("fetch outcome is not a document"),
            Self::TooLarge { bytes } => write!(formatter, "fetched document has {bytes} bytes"),
        }
    }
}

impl Error for FetchedDocumentError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParsedFeedVersion {
    Rss090,
    Rss091Userland,
    Rss092,
    Rss10,
    Rss20,
    Atom03,
    Atom10,
    JsonFeed10,
    JsonFeed11,
}

impl ParsedFeedVersion {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rss090 => "rss090",
            Self::Rss091Userland => "rss091u",
            Self::Rss092 => "rss092",
            Self::Rss10 => "rss10",
            Self::Rss20 => "rss20",
            Self::Atom03 => "atom03",
            Self::Atom10 => "atom10",
            Self::JsonFeed10 => "json10",
            Self::JsonFeed11 => "json11",
        }
    }
}

pub struct ParsedSource {
    final_url: String,
    content_type: Option<String>,
    original_encoding: String,
    source_document_hash: [u8; 32],
    etag: Option<OpaqueValidator>,
    last_modified: Option<OpaqueValidator>,
}

impl ParsedSource {
    #[must_use]
    pub fn final_url(&self) -> &str {
        &self.final_url
    }

    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    #[must_use]
    pub fn original_encoding(&self) -> &str {
        &self.original_encoding
    }

    #[must_use]
    pub const fn source_document_hash(&self) -> &[u8; 32] {
        &self.source_document_hash
    }

    #[must_use]
    pub const fn etag(&self) -> Option<&OpaqueValidator> {
        self.etag.as_ref()
    }

    #[must_use]
    pub const fn last_modified(&self) -> Option<&OpaqueValidator> {
        self.last_modified.as_ref()
    }
}

impl fmt::Debug for ParsedSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParsedSource")
            .field(
                "final_url_hash",
                &blake3::hash(self.final_url.as_bytes()).to_hex(),
            )
            .field(
                "content_type",
                &self.content_type.as_ref().map(|_| "[PRESENT]"),
            )
            .field("original_encoding", &self.original_encoding)
            .field("source_document_hash", &"[PRESENT]")
            .field("etag", &self.etag.as_ref().map(|_| "[REDACTED]"))
            .field(
                "last_modified",
                &self.last_modified.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

pub struct ParsedFeed {
    pub(crate) source: ParsedSource,
    pub(crate) version: ParsedFeedVersion,
    pub(crate) title: Option<String>,
    pub(crate) canonical_url: Option<String>,
    pub(crate) entries: Vec<ParsedEntry>,
    pub(crate) duplicate_count: usize,
}

impl ParsedFeed {
    #[must_use]
    pub const fn source(&self) -> &ParsedSource {
        &self.source
    }

    #[must_use]
    pub const fn version(&self) -> ParsedFeedVersion {
        self.version
    }

    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    #[must_use]
    pub fn canonical_url(&self) -> Option<&str> {
        self.canonical_url.as_deref()
    }

    #[must_use]
    pub fn entries(&self) -> &[ParsedEntry] {
        &self.entries
    }

    #[must_use]
    pub const fn duplicate_count(&self) -> usize {
        self.duplicate_count
    }
}

impl fmt::Debug for ParsedFeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParsedFeed")
            .field("source", &self.source)
            .field("version", &self.version)
            .field("title", &self.title.as_ref().map(|_| "[REDACTED]"))
            .field(
                "canonical_url",
                &self.canonical_url.as_ref().map(|_| "[REDACTED]"),
            )
            .field("entry_count", &self.entries.len())
            .field("duplicate_count", &self.duplicate_count)
            .finish()
    }
}

pub struct ParsedEntry {
    pub(crate) identity: EntryIdentity,
    pub(crate) canonical_url: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) content: SanitizedContent,
    pub(crate) source_content_hash: [u8; 32],
    pub(crate) content_hash: [u8; 32],
    pub(crate) published_at: Option<OffsetDateTime>,
    pub(crate) enclosures: Vec<ParsedEnclosure>,
    pub(crate) arbitration_time: Option<OffsetDateTime>,
    pub(crate) document_index: usize,
}

impl ParsedEntry {
    #[must_use]
    pub const fn identity(&self) -> &EntryIdentity {
        &self.identity
    }

    #[must_use]
    pub fn canonical_url(&self) -> Option<&str> {
        self.canonical_url.as_deref()
    }

    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    #[must_use]
    pub fn author(&self) -> Option<&str> {
        self.author.as_deref()
    }

    #[must_use]
    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    #[must_use]
    pub const fn content(&self) -> &SanitizedContent {
        &self.content
    }

    #[must_use]
    pub const fn source_content_hash(&self) -> &[u8; 32] {
        &self.source_content_hash
    }

    #[must_use]
    pub const fn content_hash(&self) -> &[u8; 32] {
        &self.content_hash
    }

    #[must_use]
    pub const fn published_at(&self) -> Option<OffsetDateTime> {
        self.published_at
    }

    #[must_use]
    pub fn enclosures(&self) -> &[ParsedEnclosure] {
        &self.enclosures
    }
}

impl fmt::Debug for ParsedEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParsedEntry")
            .field("identity", &self.identity)
            .field(
                "canonical_url",
                &self.canonical_url.as_ref().map(|_| "[REDACTED]"),
            )
            .field("title", &self.title.as_ref().map(|_| "[REDACTED]"))
            .field("author", &self.author.as_ref().map(|_| "[REDACTED]"))
            .field("summary", &self.summary.as_ref().map(|_| "[REDACTED]"))
            .field("content", &self.content)
            .field("source_content_hash", &"[PRESENT]")
            .field("content_hash", &"[PRESENT]")
            .field("published_at", &self.published_at)
            .field("enclosure_count", &self.enclosures.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ParsedEnclosure {
    pub(crate) url: String,
    pub(crate) media_type: Option<String>,
    pub(crate) length: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) duration: Option<String>,
}

impl ParsedEnclosure {
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub fn media_type(&self) -> Option<&str> {
        self.media_type.as_deref()
    }

    #[must_use]
    pub fn length(&self) -> Option<&str> {
        self.length.as_deref()
    }

    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    #[must_use]
    pub fn duration(&self) -> Option<&str> {
        self.duration.as_deref()
    }
}

impl fmt::Debug for ParsedEnclosure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParsedEnclosure")
            .field("url", &"[REDACTED]")
            .field(
                "media_type",
                &self.media_type.as_ref().map(|_| "[REDACTED]"),
            )
            .field("length", &self.length.as_ref().map(|_| "[REDACTED]"))
            .field("title", &self.title.as_ref().map(|_| "[REDACTED]"))
            .field("duration", &self.duration.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FeedParseErrorKind {
    UnsupportedContentType,
    MimeMismatch,
    UnsupportedCharset,
    DecodeFailed,
    ConvertedTooLarge,
    DoctypeForbidden,
    UnsupportedEntity,
    MalformedXml,
    DepthLimit,
    EventLimit,
    AttributeCountLimit,
    AttributeValueLimit,
    MalformedJson,
    UnsupportedVersion,
    BozoRejected,
    ParserFailure,
    ProjectedInheritanceTooLarge,
    TitleTooLong,
    ContentTooLong,
    SanitizedContentTooLong,
    TotalTextTooLong,
    TooManyEntries,
    TooManyContentBlocks,
    TooManyEnclosures,
    EnclosureJsonTooLarge,
    TooManyImages,
    ImageAltTooLong,
    ImageDimensionInvalid,
    ImageMetadataTooLarge,
    InvalidUrl,
    IdentityFailure,
    IdentityHashCollision,
    ParserBusy,
    SemaphoreClosed,
    WorkerPanicked,
}

pub struct FeedParseError {
    pub(crate) kind: FeedParseErrorKind,
    pub(crate) format: Option<ParsedFeedVersion>,
    pub(crate) count: Option<usize>,
    pub(crate) byte_length: Option<usize>,
    pub(crate) hash_present: bool,
}

impl FeedParseError {
    pub(crate) const fn new(kind: FeedParseErrorKind) -> Self {
        Self {
            kind,
            format: None,
            count: None,
            byte_length: None,
            hash_present: false,
        }
    }

    pub(crate) const fn with_count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    pub(crate) const fn with_bytes(mut self, byte_length: usize) -> Self {
        self.byte_length = Some(byte_length);
        self
    }

    pub(crate) const fn with_format(mut self, format: ParsedFeedVersion) -> Self {
        self.format = Some(format);
        self
    }

    #[must_use]
    pub const fn kind(&self) -> FeedParseErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn format(&self) -> Option<ParsedFeedVersion> {
        self.format
    }

    #[must_use]
    pub const fn count(&self) -> Option<usize> {
        self.count
    }

    #[must_use]
    pub const fn byte_length(&self) -> Option<usize> {
        self.byte_length
    }

    #[must_use]
    pub const fn hash_present(&self) -> bool {
        self.hash_present
    }
}

impl fmt::Display for FeedParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "feed parse {:?}", self.kind)?;
        if let Some(format) = self.format {
            write!(formatter, " format {}", format.as_str())?;
        }
        if let Some(count) = self.count {
            write!(formatter, " count {count}")?;
        }
        if let Some(byte_length) = self.byte_length {
            write!(formatter, " bytes {byte_length}")?;
        }
        if self.hash_present {
            formatter.write_str(" hash present")?;
        }
        Ok(())
    }
}

impl fmt::Debug for FeedParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FeedParseError")
            .field("kind", &self.kind)
            .field("format", &self.format)
            .field("count", &self.count)
            .field("byte_length", &self.byte_length)
            .field("hash_present", &self.hash_present)
            .finish()
    }
}

impl Error for FeedParseError {}

pub(crate) struct ParsedEntryCandidate {
    pub(crate) guid: Option<String>,
    pub(crate) canonical_url: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) authors: Vec<String>,
    pub(crate) summary: Option<String>,
    pub(crate) raw_content: String,
    pub(crate) content_base: Option<String>,
    pub(crate) published_at: Option<OffsetDateTime>,
    pub(crate) updated_at: Option<OffsetDateTime>,
    pub(crate) enclosures: Vec<ParsedEnclosure>,
    pub(crate) document_index: usize,
}

pub(crate) struct MappedFeed {
    pub(crate) version: ParsedFeedVersion,
    pub(crate) title: Option<String>,
    pub(crate) canonical_url: Option<String>,
    pub(crate) entries: Vec<ParsedEntryCandidate>,
}

pub(crate) fn parsed_source(
    document: FetchedDocument,
    original_encoding: String,
    source_document_hash: [u8; 32],
) -> ParsedSource {
    ParsedSource {
        final_url: document.url.complete().to_owned(),
        content_type: document.content_type,
        original_encoding,
        source_document_hash,
        etag: document.etag,
        last_modified: document.last_modified,
    }
}
