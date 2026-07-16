use std::sync::Arc;

use quick_xml::{
    Reader,
    encoding::Decoder,
    events::{BytesRef, BytesStart, Event},
};
use url::Url;

use super::{
    encoding::xml_declaration,
    types::{
        FeedParseError, FeedParseErrorKind, MAX_ATTRIBUTE_BYTES, MAX_ATTRIBUTES,
        MAX_CONTENT_BLOCKS, MAX_DEPTH, MAX_ENCLOSURES, MAX_ENTRIES, MAX_EVENTS, ParsedFeedVersion,
    },
};

const ATOM_10_NAMESPACE: &str = "http://www.w3.org/2005/Atom";
const ATOM_03_NAMESPACE: &str = "http://purl.org/atom/ns#";
const RDF_NAMESPACE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RSS_10_NAMESPACE: &str = "http://purl.org/rss/1.0/";
const MAX_FEED_LINKS: usize = 256;
const MAX_ENTRY_LINKS: usize = 256;

pub(crate) struct PreflightedXml {
    pub(crate) parser_bytes: Vec<u8>,
    pub(crate) version: ParsedFeedVersion,
    pub(crate) feed_base: Arc<str>,
    pub(crate) feed_link: Option<String>,
    pub(crate) feed_link_count: usize,
    pub(crate) entries: Vec<EntryPreflight>,
}

pub(crate) struct EntryPreflight {
    pub(crate) effective_base: Arc<str>,
    pub(crate) raw_link: Option<String>,
    pub(crate) link_count: usize,
    pub(crate) enclosure_urls: Vec<String>,
    pub(crate) summary_base: Option<Arc<str>>,
    pub(crate) content_base: Option<Arc<str>>,
    pub(crate) content_base_conflict: bool,
    pub(crate) content_count: usize,
}

struct LinkCapture {
    depth: usize,
    entry_index: Option<usize>,
    value: String,
}

struct ElementAttributes {
    values: Vec<(String, String)>,
}

impl ElementAttributes {
    fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn namespace(&self, prefix: Option<&str>) -> Option<&str> {
        match prefix {
            Some(prefix) => self.get(&format!("xmlns:{prefix}")),
            None => self.get("xmlns"),
        }
    }
}

pub(crate) fn preflight(input: &str, final_url: &str) -> Result<PreflightedXml, FeedParseError> {
    let final_url = validate_base(final_url, final_url)?;
    let without_bom = input.strip_prefix('\u{feff}').unwrap_or(input);
    let parser_text = if let Some(declaration) = xml_declaration(without_bom)? {
        &without_bom[declaration.end..]
    } else {
        without_bom
    };

    let mut reader = Reader::from_str(parser_text);
    reader.config_mut().enable_all_checks(true);
    let mut depth = 0_usize;
    let mut events = 0_usize;
    let mut root_seen = false;
    let mut root_closed = false;
    let mut version = None;
    let mut feed_base = final_url.clone();
    let mut feed_link = None;
    let mut feed_link_count = 0_usize;
    let mut entries = Vec::<EntryPreflight>::new();
    let mut base_stack = Vec::<Arc<str>>::new();
    let mut name_stack = Vec::<String>::new();
    let mut entry_stack = Vec::<(usize, usize)>::new();
    let mut link_capture: Option<LinkCapture> = None;

    loop {
        let event = reader
            .read_event()
            .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedXml))?;
        if !matches!(event, Event::Eof) {
            events += 1;
            if events > MAX_EVENTS {
                return Err(FeedParseError::new(FeedParseErrorKind::EventLimit).with_count(events));
            }
        }
        match event {
            Event::Start(element) => {
                begin_root(&mut root_seen, root_closed, depth)?;
                let attributes = collect_attributes(&element, reader.decoder())?;
                let qualified_name = element.name();
                let name = decode_name(qualified_name.as_ref())?;
                if depth == 0 {
                    version = Some(validate_signature(name, &attributes)?);
                }
                let parent_base = base_stack.last().unwrap_or(&final_url);
                let parent_name = name_stack.last().map(String::as_str);
                let current_version =
                    version.ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MimeMismatch))?;
                let recognized_entry =
                    is_recognized_entry(current_version, name, depth, parent_name);
                let direct_entry = entry_stack
                    .last()
                    .filter(|(entry_depth, _)| *entry_depth == depth)
                    .map(|(_, index)| *index);
                let role = element_role(current_version, name, depth, parent_name, direct_entry);
                let effective_base = effective_base(
                    &attributes,
                    parent_base,
                    matches!(
                        role,
                        ElementRole::FeedLink | ElementRole::EntryLink | ElementRole::Enclosure
                    ),
                )?;
                if depth == 0 || is_recognized_channel(current_version, name, depth, parent_name) {
                    feed_base = effective_base.clone();
                }
                if recognized_entry {
                    if entries.len() == MAX_ENTRIES {
                        return Err(FeedParseError::new(FeedParseErrorKind::TooManyEntries)
                            .with_count(entries.len() + 1));
                    }
                    let index = entries.len();
                    entries.push(EntryPreflight {
                        effective_base: effective_base.clone(),
                        raw_link: None,
                        link_count: 0,
                        enclosure_urls: Vec::with_capacity(1),
                        summary_base: None,
                        content_base: None,
                        content_base_conflict: false,
                        content_count: 0,
                    });
                    entry_stack.push((depth + 1, index));
                } else {
                    record_element(
                        role,
                        &attributes,
                        &effective_base,
                        &mut entries,
                        &mut feed_link,
                        &mut feed_link_count,
                        &mut link_capture,
                        depth + 1,
                    )?;
                }
                base_stack.push(effective_base);
                name_stack.push(name.to_owned());
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(
                        FeedParseError::new(FeedParseErrorKind::DepthLimit).with_count(depth)
                    );
                }
            }
            Event::Empty(element) => {
                begin_root(&mut root_seen, root_closed, depth)?;
                let attributes = collect_attributes(&element, reader.decoder())?;
                let qualified_name = element.name();
                let name = decode_name(qualified_name.as_ref())?;
                if depth == 0 {
                    version = Some(validate_signature(name, &attributes)?);
                }
                let parent_base = base_stack.last().unwrap_or(&final_url);
                let parent_name = name_stack.last().map(String::as_str);
                let current_version =
                    version.ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MimeMismatch))?;
                let direct_entry = entry_stack
                    .last()
                    .filter(|(entry_depth, _)| *entry_depth == depth)
                    .map(|(_, index)| *index);
                let role = element_role(current_version, name, depth, parent_name, direct_entry);
                let effective_base = effective_base(
                    &attributes,
                    parent_base,
                    matches!(
                        role,
                        ElementRole::FeedLink | ElementRole::EntryLink | ElementRole::Enclosure
                    ),
                )?;
                if depth == 0 || is_recognized_channel(current_version, name, depth, parent_name) {
                    feed_base = effective_base.clone();
                }
                if !is_recognized_entry(current_version, name, depth, parent_name) {
                    record_element(
                        role,
                        &attributes,
                        &effective_base,
                        &mut entries,
                        &mut feed_link,
                        &mut feed_link_count,
                        &mut None,
                        depth + 1,
                    )?;
                }
                if depth == 0 {
                    root_closed = true;
                }
            }
            Event::End(element) => {
                if depth == 0 {
                    return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
                }
                let qualified_name = element.name();
                let local = local_name(decode_name(qualified_name.as_ref())?);
                if local == "link"
                    && let Some(capture) = link_capture.take()
                {
                    if capture.depth != depth {
                        return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
                    }
                    record_link(
                        capture.value.trim(),
                        capture.entry_index,
                        &mut entries,
                        &mut feed_link,
                    );
                }
                if matches!(local, "entry" | "item")
                    && entry_stack
                        .last()
                        .is_some_and(|(entry_depth, _)| *entry_depth == depth)
                {
                    let _ = entry_stack.pop();
                }
                depth -= 1;
                let _ = base_stack.pop();
                let _ = name_stack.pop();
                if depth == 0 {
                    root_closed = true;
                }
            }
            Event::Text(text) => {
                let bytes: &[u8] = text.as_ref();
                if depth == 0 && !bytes.iter().all(u8::is_ascii_whitespace) {
                    return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
                }
                if let Some(capture) = link_capture.as_mut() {
                    capture.value.push_str(
                        &text
                            .xml10_content()
                            .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedXml))?,
                    );
                }
            }
            Event::GeneralRef(reference) => {
                let character = validate_reference(&reference)?;
                if let Some(capture) = link_capture.as_mut() {
                    capture.value.push(character);
                }
            }
            Event::DocType(_) => {
                return Err(FeedParseError::new(FeedParseErrorKind::DoctypeForbidden));
            }
            Event::Decl(_) => {
                return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
            }
            Event::CData(data) => {
                if depth == 0 {
                    return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
                }
                let decoded = data
                    .decode()
                    .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedXml))?;
                if !decoded.chars().all(xml_10_character) {
                    return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
                }
                if let Some(capture) = link_capture.as_mut() {
                    capture.value.push_str(&decoded);
                }
            }
            Event::Eof => break,
            Event::Comment(_) | Event::PI(_) => {}
        }
    }
    if !root_seen || !root_closed || depth != 0 || link_capture.is_some() {
        return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
    }
    Ok(PreflightedXml {
        parser_bytes: parser_text.as_bytes().to_vec(),
        version: version.ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MimeMismatch))?,
        feed_base,
        feed_link,
        feed_link_count,
        entries,
    })
}

#[derive(Clone, Copy)]
enum ElementRole {
    Ignore,
    FeedLink,
    EntryLink,
    Summary,
    Content,
    Enclosure,
}

fn element_role(
    version: ParsedFeedVersion,
    name: &str,
    depth: usize,
    parent_name: Option<&str>,
    direct_entry: Option<usize>,
) -> ElementRole {
    if direct_entry.is_some() {
        return match version {
            ParsedFeedVersion::Atom03 | ParsedFeedVersion::Atom10 => match name {
                "link" => ElementRole::EntryLink,
                "summary" => ElementRole::Summary,
                "content" => ElementRole::Content,
                _ => ElementRole::Ignore,
            },
            ParsedFeedVersion::Rss090
            | ParsedFeedVersion::Rss091Userland
            | ParsedFeedVersion::Rss092
            | ParsedFeedVersion::Rss20 => match name {
                "link" => ElementRole::EntryLink,
                "description" => ElementRole::Summary,
                "enclosure" => ElementRole::Enclosure,
                "content:encoded" => ElementRole::Content,
                _ => ElementRole::Ignore,
            },
            ParsedFeedVersion::Rss10 => match local_name(name) {
                "link" => ElementRole::EntryLink,
                "description" => ElementRole::Summary,
                "encoded" if name.contains(':') => ElementRole::Content,
                _ => ElementRole::Ignore,
            },
            ParsedFeedVersion::JsonFeed10 | ParsedFeedVersion::JsonFeed11 => ElementRole::Ignore,
        };
    }

    let direct_feed_child = match version {
        ParsedFeedVersion::Atom03 | ParsedFeedVersion::Atom10 => {
            depth == 1 && parent_name.is_some_and(|parent| local_name(parent) == "feed")
        }
        ParsedFeedVersion::Rss090
        | ParsedFeedVersion::Rss091Userland
        | ParsedFeedVersion::Rss092
        | ParsedFeedVersion::Rss20 => depth == 2 && parent_name == Some("channel"),
        ParsedFeedVersion::Rss10 => {
            depth == 2 && parent_name.is_some_and(|parent| local_name(parent) == "channel")
        }
        ParsedFeedVersion::JsonFeed10 | ParsedFeedVersion::JsonFeed11 => false,
    };
    if direct_feed_child
        && match version {
            ParsedFeedVersion::Rss10 => local_name(name) == "link",
            ParsedFeedVersion::Rss090
            | ParsedFeedVersion::Rss091Userland
            | ParsedFeedVersion::Rss092
            | ParsedFeedVersion::Rss20 => {
                matches!(name, "link" | "atom:link" | "atom10:link")
            }
            _ => name == "link",
        }
    {
        ElementRole::FeedLink
    } else {
        ElementRole::Ignore
    }
}

fn is_recognized_entry(
    version: ParsedFeedVersion,
    name: &str,
    depth: usize,
    parent_name: Option<&str>,
) -> bool {
    match version {
        ParsedFeedVersion::Atom03 | ParsedFeedVersion::Atom10 => {
            depth == 1
                && name == "entry"
                && parent_name.is_some_and(|parent| local_name(parent) == "feed")
        }
        ParsedFeedVersion::Rss090
        | ParsedFeedVersion::Rss091Userland
        | ParsedFeedVersion::Rss092
        | ParsedFeedVersion::Rss20 => {
            depth == 2 && name == "item" && parent_name == Some("channel")
        }
        ParsedFeedVersion::Rss10 => {
            depth == 1
                && local_name(name) == "item"
                && parent_name.is_some_and(|parent| local_name(parent) == "RDF")
        }
        ParsedFeedVersion::JsonFeed10 | ParsedFeedVersion::JsonFeed11 => false,
    }
}

fn is_recognized_channel(
    version: ParsedFeedVersion,
    name: &str,
    depth: usize,
    parent_name: Option<&str>,
) -> bool {
    match version {
        ParsedFeedVersion::Rss090
        | ParsedFeedVersion::Rss091Userland
        | ParsedFeedVersion::Rss092
        | ParsedFeedVersion::Rss20 => depth == 1 && name == "channel",
        ParsedFeedVersion::Rss10 => {
            depth == 1
                && local_name(name) == "channel"
                && parent_name.is_some_and(|parent| local_name(parent) == "RDF")
        }
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn record_element(
    role: ElementRole,
    attributes: &ElementAttributes,
    effective_base: &Arc<str>,
    entries: &mut [EntryPreflight],
    feed_link: &mut Option<String>,
    feed_link_count: &mut usize,
    link_capture: &mut Option<LinkCapture>,
    element_depth: usize,
) -> Result<(), FeedParseError> {
    let entry_index = match role {
        ElementRole::EntryLink
        | ElementRole::Summary
        | ElementRole::Content
        | ElementRole::Enclosure => entries.len().checked_sub(1),
        ElementRole::Ignore | ElementRole::FeedLink => None,
    };
    match role {
        ElementRole::Ignore => {}
        ElementRole::FeedLink => {
            increment_count(
                feed_link_count,
                MAX_FEED_LINKS,
                FeedParseErrorKind::ParserFailure,
            )?;
            if let Some(href) = attributes.get("href") {
                if attributes.get("rel").is_none_or(|rel| rel == "alternate") {
                    record_link(href, None, entries, feed_link);
                }
            } else {
                *link_capture = Some(LinkCapture {
                    depth: element_depth,
                    entry_index: None,
                    value: String::new(),
                });
            }
        }
        ElementRole::EntryLink => {
            let index = entry_index.expect("entry role has an entry");
            increment_count(
                &mut entries[index].link_count,
                MAX_ENTRY_LINKS,
                FeedParseErrorKind::ParserFailure,
            )?;
            if let Some(href) = attributes.get("href") {
                if attributes.get("rel") == Some("enclosure") {
                    record_enclosure(index, href, entries)?;
                } else if attributes.get("rel").is_none_or(|rel| rel == "alternate") {
                    record_link(href, Some(index), entries, feed_link);
                }
            } else {
                *link_capture = Some(LinkCapture {
                    depth: element_depth,
                    entry_index: Some(index),
                    value: String::new(),
                });
            }
        }
        ElementRole::Summary => {
            let index = entry_index.expect("entry role has an entry");
            if entries[index].summary_base.is_none() {
                entries[index].summary_base = Some(effective_base.clone());
            }
        }
        ElementRole::Content => {
            let index = entry_index.expect("entry role has an entry");
            let entry = &mut entries[index];
            increment_count(
                &mut entry.content_count,
                MAX_CONTENT_BLOCKS,
                FeedParseErrorKind::TooManyContentBlocks,
            )?;
            if let Some(first) = entry.content_base.as_ref() {
                entry.content_base_conflict |= first.as_ref() != effective_base.as_ref();
            } else {
                entry.content_base = Some(effective_base.clone());
            }
        }
        ElementRole::Enclosure => {
            let index = entry_index.expect("entry role has an entry");
            increment_count(
                &mut entries[index].link_count,
                MAX_ENTRY_LINKS,
                FeedParseErrorKind::ParserFailure,
            )?;
            if let Some(url) = attributes.get("url") {
                record_enclosure(index, url, entries)?;
            }
        }
    }
    Ok(())
}

fn record_enclosure(
    index: usize,
    url: &str,
    entries: &mut [EntryPreflight],
) -> Result<(), FeedParseError> {
    if entries[index].enclosure_urls.len() == MAX_ENCLOSURES {
        return Err(FeedParseError::new(FeedParseErrorKind::TooManyEnclosures)
            .with_count(entries[index].enclosure_urls.len() + 1));
    }
    entries[index].enclosure_urls.push(url.to_owned());
    Ok(())
}

fn increment_count(
    count: &mut usize,
    maximum: usize,
    kind: FeedParseErrorKind,
) -> Result<(), FeedParseError> {
    let next = count.saturating_add(1);
    if next > maximum {
        return Err(FeedParseError::new(kind).with_count(next));
    }
    *count = next;
    Ok(())
}

fn record_link(
    value: &str,
    entry_index: Option<usize>,
    entries: &mut [EntryPreflight],
    feed_link: &mut Option<String>,
) {
    if value.is_empty() {
        return;
    }
    if let Some(index) = entry_index {
        if entries[index].raw_link.is_none() {
            entries[index].raw_link = Some(value.to_owned());
        }
    } else if feed_link.is_none() {
        *feed_link = Some(value.to_owned());
    }
}

fn validate_signature(
    name: &str,
    attributes: &ElementAttributes,
) -> Result<ParsedFeedVersion, FeedParseError> {
    let (prefix, local) = split_name(name);
    match local {
        "rss" if prefix.is_none() => match attributes.get("version") {
            Some("0.90") => Ok(ParsedFeedVersion::Rss090),
            Some("0.91") => Ok(ParsedFeedVersion::Rss091Userland),
            Some("0.92") => Ok(ParsedFeedVersion::Rss092),
            Some("2.0") => Ok(ParsedFeedVersion::Rss20),
            _ => Err(FeedParseError::new(FeedParseErrorKind::UnsupportedVersion)),
        },
        "feed" => match attributes.namespace(prefix) {
            Some(ATOM_10_NAMESPACE) => Ok(ParsedFeedVersion::Atom10),
            Some(ATOM_03_NAMESPACE) => Ok(ParsedFeedVersion::Atom03),
            _ => Err(FeedParseError::new(FeedParseErrorKind::UnsupportedVersion)),
        },
        "RDF" if attributes.namespace(prefix) == Some(RDF_NAMESPACE) => {
            if attributes
                .values
                .iter()
                .any(|(key, value)| key.starts_with("xmlns") && value == RSS_10_NAMESPACE)
            {
                Ok(ParsedFeedVersion::Rss10)
            } else {
                Err(FeedParseError::new(FeedParseErrorKind::UnsupportedVersion))
            }
        }
        _ => Err(FeedParseError::new(FeedParseErrorKind::MimeMismatch)),
    }
}

fn effective_base(
    attributes: &ElementAttributes,
    parent: &Arc<str>,
    leaf_base_forbidden: bool,
) -> Result<Arc<str>, FeedParseError> {
    let Some(explicit) = attributes.get("xml:base") else {
        return Ok(parent.clone());
    };
    if leaf_base_forbidden {
        return Err(FeedParseError::new(FeedParseErrorKind::InvalidUrl));
    }
    validate_base(explicit, parent)
}

fn validate_base(raw: &str, parent: &str) -> Result<Arc<str>, FeedParseError> {
    let parent =
        Url::parse(parent).map_err(|_| FeedParseError::new(FeedParseErrorKind::InvalidUrl))?;
    let mut base = parent
        .join(raw)
        .map_err(|_| FeedParseError::new(FeedParseErrorKind::InvalidUrl))?;
    if !matches!(base.scheme(), "http" | "https")
        || base.host().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
    {
        return Err(FeedParseError::new(FeedParseErrorKind::InvalidUrl));
    }
    base.set_fragment(None);
    let normalized = base.to_string();
    if normalized.len() > 4_096 {
        return Err(
            FeedParseError::new(FeedParseErrorKind::InvalidUrl).with_bytes(normalized.len())
        );
    }
    Ok(Arc::from(normalized))
}

fn begin_root(root_seen: &mut bool, root_closed: bool, depth: usize) -> Result<(), FeedParseError> {
    if depth == 0 {
        if *root_seen || root_closed {
            return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
        }
        *root_seen = true;
    }
    Ok(())
}

fn collect_attributes(
    element: &BytesStart<'_>,
    decoder: Decoder,
) -> Result<ElementAttributes, FeedParseError> {
    let mut values = Vec::new();
    for attribute in element.attributes().with_checks(true) {
        let attribute =
            attribute.map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedXml))?;
        if values.len() == MAX_ATTRIBUTES {
            return Err(FeedParseError::new(FeedParseErrorKind::AttributeCountLimit)
                .with_count(values.len() + 1));
        }
        if attribute.value.len() > MAX_ATTRIBUTE_BYTES {
            return Err(FeedParseError::new(FeedParseErrorKind::AttributeValueLimit)
                .with_bytes(attribute.value.len()));
        }
        validate_attribute_references(&attribute.value)?;
        let key = decode_name(attribute.key.as_ref())?.to_owned();
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
            .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedXml))?
            .into_owned();
        values.push((key, value));
    }
    Ok(ElementAttributes { values })
}

fn validate_reference(reference: &BytesRef<'_>) -> Result<char, FeedParseError> {
    if reference.is_char_ref() {
        let character = reference
            .resolve_char_ref()
            .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedXml))?
            .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedXml))?;
        if xml_10_character(character) {
            return Ok(character);
        }
        return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
    }
    let reference: &[u8] = reference.as_ref();
    match reference {
        b"amp" => Ok('&'),
        b"lt" => Ok('<'),
        b"gt" => Ok('>'),
        b"apos" => Ok('\''),
        b"quot" => Ok('"'),
        _ => Err(FeedParseError::new(FeedParseErrorKind::UnsupportedEntity)),
    }
}

fn validate_attribute_references(raw: &[u8]) -> Result<(), FeedParseError> {
    let mut cursor = 0_usize;
    while let Some(relative) = raw[cursor..].iter().position(|byte| *byte == b'&') {
        let start = cursor + relative + 1;
        let end = raw[start..]
            .iter()
            .position(|byte| *byte == b';')
            .map(|relative| start + relative)
            .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedXml))?;
        let reference = BytesRef::new(
            std::str::from_utf8(&raw[start..end])
                .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedXml))?,
        );
        let _ = validate_reference(&reference)?;
        cursor = end + 1;
    }
    Ok(())
}

fn xml_10_character(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\r')
        || matches!(character as u32, 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff)
}

fn decode_name(bytes: &[u8]) -> Result<&str, FeedParseError> {
    std::str::from_utf8(bytes).map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedXml))
}

fn split_name(name: &str) -> (Option<&str>, &str) {
    name.rsplit_once(':')
        .map_or((None, name), |(prefix, local)| (Some(prefix), local))
}

fn local_name(name: &str) -> &str {
    split_name(name).1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_bytes_only_drop_the_leading_declaration() {
        let input = "<?xml version='1.0'?><rss version='2.0' a='&amp;'><![CDATA[ x ]]></rss>";
        let output = preflight(input, "https://example.test/feed.xml").expect("valid XML");
        assert_eq!(
            output.parser_bytes,
            b"<rss version='2.0' a='&amp;'><![CDATA[ x ]]></rss>"
        );
    }

    #[test]
    fn event_limit_accepts_n_and_rejects_n_plus_one() {
        fn document(empty_events: usize) -> String {
            let mut xml = String::with_capacity(empty_events * 4 + 128);
            xml.push_str("<rss version='2.0'><channel><title>x</title>");
            for _ in 0..empty_events {
                xml.push_str("<x/>");
            }
            xml.push_str("</channel></rss>");
            xml
        }

        preflight(&document(999_993), "https://example.test/feed.xml")
            .expect("exactly 1,000,000 events is accepted");
        let error = preflight(&document(999_994), "https://example.test/feed.xml")
            .err()
            .expect("1,000,001 events rejects");
        assert_eq!(error.kind(), FeedParseErrorKind::EventLimit);
        assert_eq!(error.count(), Some(1_000_001));
    }

    #[test]
    fn xml_10_numeric_reference_legal_character_boundaries_are_frozen() {
        let valid = "<rss version='2.0'><channel><title>&#9;&#10;&#13;&#32;&#xD7FF;&#xE000;&#xFFFD;&#x10000;&#x10FFFF;</title></channel></rss>";
        preflight(valid, "https://example.test/feed.xml").expect("XML 1.0 legal refs");
        for reference in ["&#0;", "&#xB;", "&#xD800;", "&#xFFFE;", "&#x110000;"] {
            let xml =
                format!("<rss version='2.0'><channel><title>{reference}</title></channel></rss>");
            let error = preflight(&xml, "https://example.test/feed.xml")
                .err()
                .expect("illegal XML 1.0 reference rejects");
            assert_eq!(error.kind(), FeedParseErrorKind::MalformedXml);
        }
    }

    #[test]
    fn sidecar_content_bases_reject_before_inherited_base_amplification() {
        fn document(content_blocks: usize) -> String {
            let long_base = format!("https://example.test/{}/", "b".repeat(3_900));
            let mut xml = format!(
                "<feed xmlns='http://www.w3.org/2005/Atom' xml:base='{long_base}'><title>x</title><id>x</id><updated>2026-07-16T00:00:00Z</updated><entry><id>x</id><updated>2026-07-16T00:00:00Z</updated>"
            );
            for _ in 0..20_000 {
                xml.push_str("<extension/>");
            }
            for _ in 0..content_blocks {
                xml.push_str("<content type='text'>x</content>");
            }
            xml.push_str("</entry></feed>");
            xml
        }

        let accepted = preflight(&document(64), "https://example.test/feed.xml")
            .expect("64 content bases are bounded");
        assert_eq!(accepted.entries[0].content_count, 64);
        assert!(!accepted.entries[0].content_base_conflict);

        let error = preflight(&document(65), "https://example.test/feed.xml")
            .err()
            .expect("65th content base rejects before sidecar allocation grows");
        assert_eq!(error.kind(), FeedParseErrorKind::TooManyContentBlocks);
        assert_eq!(error.count(), Some(65));
    }
}
