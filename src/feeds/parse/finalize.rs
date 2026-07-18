use std::collections::HashMap;

use crate::{
    content::{
        SanitizeError, resanitize_entry_html,
        sanitize::{content_hash, source_content_hash},
        sanitize_entry_html,
    },
    feeds::{EntryIdentity, StableEntryFields},
};

use super::types::{
    FeedParseError, FeedParseErrorKind, MAX_TOTAL_TEXT_BYTES, ParsedEntry, ParsedEntryCandidate,
};

pub(crate) fn finalize(
    candidates: Vec<ParsedEntryCandidate>,
) -> Result<(Vec<ParsedEntry>, usize), FeedParseError> {
    finalize_with_index_key(candidates, |identity| identity.index_hash().to_owned())
}

fn finalize_with_index_key(
    candidates: Vec<ParsedEntryCandidate>,
    index_key: impl Fn(&EntryIdentity) -> String,
) -> Result<(Vec<ParsedEntry>, usize), FeedParseError> {
    let mut entries = Vec::<ParsedEntry>::with_capacity(candidates.len());
    let mut positions = HashMap::<String, usize>::with_capacity(candidates.len());
    let mut duplicate_count = 0_usize;

    for candidate in candidates {
        let base = candidate
            .content_base
            .as_deref()
            .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::InvalidUrl))?;
        let core = sanitize_entry_html(base, &candidate.raw_content).map_err(map_sanitize_error)?;
        let source_hash = source_content_hash(core.html());

        // Concrete no-processor path. This is the future async content-processing insertion point.
        let final_content =
            resanitize_entry_html(base, core.html(), core.images()).map_err(map_sanitize_error)?;
        let final_hash = content_hash(final_content.html());
        check_total_text(&candidate, final_content.html())?;

        let published_at = candidate.published_at.or(candidate.updated_at);
        let arbitration_time = candidate.updated_at.or(candidate.published_at);
        let published_at_us = published_at
            .map(time::OffsetDateTime::unix_timestamp_nanos)
            .and_then(|nanoseconds| i64::try_from(nanoseconds / 1_000).ok());
        let stable_fields = StableEntryFields::new(
            candidate.title.as_deref(),
            candidate.author.as_deref(),
            published_at_us,
            candidate
                .enclosures
                .first()
                .map(|enclosure| enclosure.url.as_str()),
            Some(final_hash),
        )
        .map_err(|_| FeedParseError::new(FeedParseErrorKind::IdentityFailure))?;
        let identity = EntryIdentity::from_parts(
            candidate.guid.as_deref(),
            candidate.canonical_url.as_deref(),
            stable_fields,
        )
        .map_err(|_| FeedParseError::new(FeedParseErrorKind::IdentityFailure))?;
        let entry = ParsedEntry {
            identity,
            canonical_url: candidate.canonical_url,
            title: candidate.title,
            author: candidate.author,
            summary: candidate.summary,
            content: final_content,
            source_content_hash: source_hash,
            content_hash: final_hash,
            published_at,
            enclosures: candidate.enclosures,
            arbitration_time,
            document_index: candidate.document_index,
        };

        let key = index_key(&entry.identity);
        if let Some(position) = positions.get(&key).copied() {
            duplicate_count += 1;
            let existing = &entries[position];
            if existing.identity.kind() != entry.identity.kind()
                || existing.identity.identity() != entry.identity.identity()
            {
                return Err(FeedParseError::new(
                    FeedParseErrorKind::IdentityHashCollision,
                ));
            }
            if newer(entry.arbitration_time, existing.arbitration_time) {
                let first_document_index = existing.document_index;
                let mut replacement = entry;
                replacement.document_index = first_document_index;
                entries[position] = replacement;
            }
        } else {
            positions.insert(key, entries.len());
            entries.push(entry);
        }
    }
    Ok((entries, duplicate_count))
}

fn newer(candidate: Option<time::OffsetDateTime>, existing: Option<time::OffsetDateTime>) -> bool {
    match (candidate, existing) {
        (Some(candidate), Some(existing)) => candidate > existing,
        (Some(_), None) => true,
        (None, Some(_) | None) => false,
    }
}

fn check_total_text(
    candidate: &super::types::ParsedEntryCandidate,
    final_html: &str,
) -> Result<(), FeedParseError> {
    let enclosure_bytes = candidate
        .enclosures
        .iter()
        .map(|enclosure| {
            enclosure.url.len()
                + enclosure.media_type.as_ref().map_or(0, String::len)
                + enclosure.length.as_ref().map_or(0, String::len)
                + enclosure.title.as_ref().map_or(0, String::len)
                + enclosure.duration.as_ref().map_or(0, String::len)
        })
        .sum::<usize>();
    let bytes = candidate.guid.as_ref().map_or(0, String::len)
        + candidate.canonical_url.as_ref().map_or(0, String::len)
        + candidate.title.as_ref().map_or(0, String::len)
        + candidate.authors.iter().map(String::len).sum::<usize>()
        + candidate.summary.as_ref().map_or(0, String::len)
        + candidate.raw_content.len()
        + final_html.len()
        + enclosure_bytes;
    if bytes > MAX_TOTAL_TEXT_BYTES {
        Err(FeedParseError::new(FeedParseErrorKind::TotalTextTooLong).with_bytes(bytes))
    } else {
        Ok(())
    }
}

fn map_sanitize_error(error: SanitizeError) -> FeedParseError {
    match error {
        SanitizeError::FinalHtmlTooLong { bytes } => {
            FeedParseError::new(FeedParseErrorKind::SanitizedContentTooLong).with_bytes(bytes)
        }
        SanitizeError::TooManyImages { count } => {
            FeedParseError::new(FeedParseErrorKind::TooManyImages).with_count(count)
        }
        SanitizeError::ImageAltTooLong { bytes } => {
            FeedParseError::new(FeedParseErrorKind::ImageAltTooLong).with_bytes(bytes)
        }
        SanitizeError::ImageDimensionInvalid => {
            FeedParseError::new(FeedParseErrorKind::ImageDimensionInvalid)
        }
        SanitizeError::ImageMetadataTooLarge { bytes } => {
            FeedParseError::new(FeedParseErrorKind::ImageMetadataTooLarge).with_bytes(bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feeds::parse::types::ParsedEntryCandidate;

    #[test]
    fn synthetic_index_collision_rejects_different_full_identities() {
        let candidate = |guid: &str, index| ParsedEntryCandidate {
            guid: Some(guid.to_owned()),
            canonical_url: None,
            title: None,
            author: None,
            authors: Vec::new(),
            summary: None,
            raw_content: "<p>x</p>".to_owned(),
            content_base: Some("https://example.test/".to_owned()),
            published_at: None,
            updated_at: None,
            enclosures: Vec::new(),
            document_index: index,
        };
        let error = finalize_with_index_key(vec![candidate("one", 0), candidate("two", 1)], |_| {
            "collision".to_owned()
        })
        .expect_err("different identities sharing a lookup hash reject");
        assert_eq!(error.kind(), FeedParseErrorKind::IdentityHashCollision);
    }

    #[test]
    fn total_normalized_text_budget_counts_all_owned_candidate_and_final_strings() {
        let candidate = ParsedEntryCandidate {
            guid: Some("g".repeat(64 * 1024)),
            canonical_url: None,
            title: None,
            author: Some("a".repeat(1024 * 1024)),
            authors: vec!["a".repeat(1024 * 1024)],
            summary: Some("s".repeat(1024 * 1024)),
            raw_content: format!("<script>{}</script>", "x".repeat(15 * 1024 * 1024)),
            content_base: Some("https://example.test/".to_owned()),
            published_at: None,
            updated_at: None,
            enclosures: Vec::new(),
            document_index: 0,
        };
        let error = finalize(vec![candidate]).expect_err("aggregate budget rejects");
        assert_eq!(error.kind(), FeedParseErrorKind::TotalTextTooLong);
    }
}
