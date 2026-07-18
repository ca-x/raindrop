use feedparser_rs::{FeedVersion, ParserLimits, TextType};
use time::OffsetDateTime;
use url::Url;

use super::types::{
    FeedParseError, FeedParseErrorKind, MAX_CONTENT_BLOCKS, MAX_CONTENT_BYTES,
    MAX_ENCLOSURE_JSON_BYTES, MAX_ENCLOSURES, MAX_ENTRIES, MAX_TITLE_BYTES, MappedFeed,
    ParsedEnclosure, ParsedEntryCandidate, ParsedFeedVersion,
};
use super::xml::{EntryPreflight, PreflightedXml};

pub(crate) fn parser_limits() -> ParserLimits {
    ParserLimits {
        max_entries: MAX_ENTRIES,
        max_links_per_feed: 256,
        max_links_per_entry: 256,
        max_authors: 256,
        max_contributors: 256,
        max_tags: 256,
        max_content_blocks: MAX_CONTENT_BLOCKS + 1,
        max_enclosures: MAX_ENCLOSURES + 1,
        max_namespaces: 256,
        max_nesting_depth: super::types::MAX_DEPTH,
        max_text_length: MAX_CONTENT_BYTES + 1,
        max_feed_size_bytes: super::types::MAX_DOCUMENT_BYTES,
        max_attribute_length: super::types::MAX_ATTRIBUTE_BYTES,
        max_podcast_soundbites: 256,
        max_podcast_transcripts: 256,
        max_podcast_funding: 256,
        max_podcast_persons: 256,
        max_value_recipients: 256,
        max_podcast_alternate_enclosures: 256,
        max_podcast_alternate_enclosure_sources: 256,
        max_podcast_podroll: 256,
        max_podcast_social_interact: 256,
        max_podcast_txt: 256,
        max_podcast_follow: 256,
    }
}

pub(crate) fn map_feed(
    parsed: feedparser_rs::ParsedFeed,
    final_url: &str,
    xml: Option<&PreflightedXml>,
) -> Result<MappedFeed, FeedParseError> {
    let version = map_version(parsed.version)?;
    if xml.is_some_and(|preflight| preflight.version != version) {
        return Err(FeedParseError::new(FeedParseErrorKind::UnsupportedVersion));
    }
    if parsed.bozo
        || parsed
            .bozo_exception
            .as_ref()
            .is_some_and(|value| !value.is_empty())
    {
        return Err(FeedParseError::new(FeedParseErrorKind::BozoRejected).with_format(version));
    }
    if parsed.entries.len() > MAX_ENTRIES {
        return Err(FeedParseError::new(FeedParseErrorKind::TooManyEntries)
            .with_count(parsed.entries.len())
            .with_format(version));
    }
    if let Some(preflight) = xml {
        if preflight.entries.len() != parsed.entries.len() {
            return Err(FeedParseError::new(FeedParseErrorKind::ParserFailure)
                .with_count(preflight.entries.len()));
        }
        if preflight.feed_link_count != parsed.feed.links.len() {
            return Err(FeedParseError::new(FeedParseErrorKind::ParserFailure)
                .with_count(preflight.feed_link_count));
        }
    }
    let title = normalize_text(parsed.feed.title.as_deref());
    check_title(title.as_deref())?;
    let feed_base = xml.map_or(final_url, |preflight| preflight.feed_base.as_ref());
    let canonical_url = xml
        .and_then(|preflight| preflight.feed_link.as_deref())
        .or(parsed.feed.link.as_deref())
        .and_then(|raw| normalize_url(raw, feed_base));
    let entries = parsed
        .entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let entry_preflight = xml.map(|preflight| &preflight.entries[index]);
            map_entry(entry, feed_base, entry_preflight, index)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MappedFeed {
        version,
        title,
        canonical_url,
        entries,
    })
}

fn map_version(version: FeedVersion) -> Result<ParsedFeedVersion, FeedParseError> {
    match version {
        FeedVersion::Rss090 => Ok(ParsedFeedVersion::Rss090),
        FeedVersion::Rss091Userland => Ok(ParsedFeedVersion::Rss091Userland),
        FeedVersion::Rss092 => Ok(ParsedFeedVersion::Rss092),
        FeedVersion::Rss10 => Ok(ParsedFeedVersion::Rss10),
        FeedVersion::Rss20 => Ok(ParsedFeedVersion::Rss20),
        FeedVersion::Atom03 => Ok(ParsedFeedVersion::Atom03),
        FeedVersion::Atom10 => Ok(ParsedFeedVersion::Atom10),
        FeedVersion::JsonFeed10 => Ok(ParsedFeedVersion::JsonFeed10),
        FeedVersion::JsonFeed11 => Ok(ParsedFeedVersion::JsonFeed11),
        FeedVersion::Rss091Netscape | FeedVersion::Unknown => {
            Err(FeedParseError::new(FeedParseErrorKind::UnsupportedVersion))
        }
    }
}

fn map_entry(
    entry: feedparser_rs::Entry,
    feed_base: &str,
    entry_preflight: Option<&EntryPreflight>,
    document_index: usize,
) -> Result<ParsedEntryCandidate, FeedParseError> {
    if let Some(preflight) = entry_preflight
        && (preflight.content_count != entry.content.len()
            || preflight.enclosure_urls.len() != entry.enclosures.len()
            || !entry_link_count_matches(preflight, &entry))
    {
        return Err(
            FeedParseError::new(FeedParseErrorKind::ParserFailure).with_count(document_index)
        );
    }
    let entry_base =
        entry_preflight.map_or(feed_base, |preflight| preflight.effective_base.as_ref());
    let feedparser_rs::Entry {
        id,
        title,
        link,
        summary,
        summary_detail,
        content,
        published,
        updated,
        author,
        authors,
        dc_creator,
        enclosures,
        ..
    } = entry;
    if content.len() > MAX_CONTENT_BLOCKS {
        return Err(
            FeedParseError::new(FeedParseErrorKind::TooManyContentBlocks).with_count(content.len()),
        );
    }
    if enclosures.len() > MAX_ENCLOSURES {
        return Err(
            FeedParseError::new(FeedParseErrorKind::TooManyEnclosures).with_count(enclosures.len())
        );
    }
    let title = normalize_text(title.as_deref());
    check_title(title.as_deref())?;
    let mut all_authors: Vec<String> = authors
        .iter()
        .filter_map(|person| normalize_text(person.name.as_deref()))
        .collect();
    let author = normalize_text(author.as_deref().or(dc_creator.as_deref()))
        .or_else(|| all_authors.first().cloned());
    if all_authors.is_empty()
        && let Some(author) = author.as_ref()
    {
        all_authors.push(author.clone());
    }
    let canonical_url = entry_preflight
        .and_then(|preflight| preflight.raw_link.as_deref())
        .or(link.as_deref())
        .and_then(|raw| normalize_url(raw, entry_base));

    if let Some(summary) = summary.as_ref()
        && summary.len() > MAX_CONTENT_BYTES
    {
        return Err(
            FeedParseError::new(FeedParseErrorKind::ContentTooLong).with_bytes(summary.len())
        );
    }
    let summary_for_field = summary.clone();
    let (raw_content, content_base) = if content.is_empty() {
        let summary_content = summary.clone().unwrap_or_default();
        if summary_content.len() > MAX_CONTENT_BYTES {
            return Err(FeedParseError::new(FeedParseErrorKind::ContentTooLong)
                .with_bytes(summary_content.len()));
        }
        let plain = summary_detail
            .as_ref()
            .is_some_and(|detail| detail.content_type == TextType::Text);
        let value = if plain {
            escape_html(&summary_content)
        } else {
            summary_content
        };
        let base = if let Some(preflight) = entry_preflight {
            preflight
                .summary_base
                .as_ref()
                .map(ToString::to_string)
                .or_else(|| Some(preflight.effective_base.to_string()))
        } else {
            summary_detail
                .and_then(|detail| detail.base)
                .map(|base| validate_detail_base(&base, entry_base))
                .transpose()?
                .or_else(|| Some(entry_base.to_owned()))
        };
        (value, base)
    } else {
        let mut joined = String::new();
        let mut base = None;
        for (index, block) in content.into_iter().enumerate() {
            if block.value.len() > MAX_CONTENT_BYTES {
                return Err(FeedParseError::new(FeedParseErrorKind::ContentTooLong)
                    .with_bytes(block.value.len()));
            }
            if index != 0 {
                joined.push('\n');
            }
            let plain = block
                .content_type
                .as_ref()
                .is_some_and(|content_type| content_type.as_str() == "text/plain");
            if plain {
                joined.push_str(&escape_html(&block.value));
            } else {
                joined.push_str(&block.value);
            }
            if base.is_none() {
                base = block
                    .base
                    .map(|base| validate_detail_base(&base, entry_base))
                    .transpose()?;
            }
        }
        if let Some(preflight) = entry_preflight {
            if preflight.content_base_conflict {
                return Err(FeedParseError::new(FeedParseErrorKind::InvalidUrl));
            }
            base = preflight
                .content_base
                .as_ref()
                .map(ToString::to_string)
                .or_else(|| Some(preflight.effective_base.to_string()));
        } else if base.is_none() {
            base = Some(entry_base.to_owned());
        }
        (joined, base)
    };
    let summary = summary_for_field
        .as_deref()
        .map(ammonia::clean_text)
        .and_then(|value| normalize_text(Some(&value)));
    let published_at = published.and_then(|value| {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(value.timestamp_micros()) * 1_000).ok()
    });
    let updated_at = updated.and_then(|value| {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(value.timestamp_micros()) * 1_000).ok()
    });
    let raw_enclosures = entry_preflight.map(|preflight| preflight.enclosure_urls.as_slice());
    let enclosures = enclosures
        .into_iter()
        .enumerate()
        .filter_map(|enclosure| {
            let (index, enclosure) = enclosure;
            let raw_url = raw_enclosures
                .and_then(|urls| urls.get(index))
                .map_or_else(|| enclosure.url.as_str(), String::as_str);
            let url = normalize_url(raw_url, entry_base)?;
            Some(ParsedEnclosure {
                url,
                media_type: enclosure.enclosure_type.map(|value| value.to_string()),
                length: enclosure.length,
                title: enclosure.title,
                duration: enclosure.duration,
            })
        })
        .collect::<Vec<_>>();
    check_enclosure_json(&enclosures)?;
    Ok(ParsedEntryCandidate {
        guid: id.map(|value| value.to_string()),
        canonical_url,
        title,
        author,
        authors: all_authors,
        summary,
        raw_content,
        content_base,
        published_at,
        updated_at,
        enclosures,
        document_index,
    })
}

fn entry_link_count_matches(preflight: &EntryPreflight, entry: &feedparser_rs::Entry) -> bool {
    if preflight.link_count == entry.links.len() {
        return true;
    }
    // feedparser-rs can synthesize one alternate Link from an RSS GUID or Atom ID.
    // The sidecar counts only source link/enclosure elements.
    entry.id.is_some() && entry.links.len() == preflight.link_count.saturating_add(1)
}

fn check_title(title: Option<&str>) -> Result<(), FeedParseError> {
    if let Some(title) = title
        && title.len() > MAX_TITLE_BYTES
    {
        return Err(FeedParseError::new(FeedParseErrorKind::TitleTooLong).with_bytes(title.len()));
    }
    Ok(())
}

fn check_enclosure_json(enclosures: &[ParsedEnclosure]) -> Result<(), FeedParseError> {
    let value: Vec<_> = enclosures
        .iter()
        .map(|enclosure| {
            serde_json::json!({
                "duration": enclosure.duration,
                "length": enclosure.length,
                "media_type": enclosure.media_type,
                "title": enclosure.title,
                "url": enclosure.url,
            })
        })
        .collect();
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| FeedParseError::new(FeedParseErrorKind::ParserFailure))?;
    if bytes.len() > MAX_ENCLOSURE_JSON_BYTES {
        Err(FeedParseError::new(FeedParseErrorKind::EnclosureJsonTooLarge).with_bytes(bytes.len()))
    } else {
        Ok(())
    }
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn normalize_url(raw: &str, base: &str) -> Option<String> {
    let base = Url::parse(base).ok()?;
    let mut url = base.join(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_fragment(None);
    let normalized = url.to_string();
    (normalized.len() <= 4_096).then_some(normalized)
}

fn validate_detail_base(raw: &str, parent: &str) -> Result<String, FeedParseError> {
    normalize_url(raw, parent).ok_or_else(|| FeedParseError::new(FeedParseErrorKind::InvalidUrl))
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}
