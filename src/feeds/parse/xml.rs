use quick_xml::{
    Reader,
    encoding::Decoder,
    events::{BytesRef, BytesStart, Event},
};
use url::Url;

use super::{
    encoding::xml_declaration,
    types::{
        FeedParseError, FeedParseErrorKind, MAX_ATTRIBUTE_BYTES, MAX_ATTRIBUTES, MAX_DEPTH,
        MAX_EVENTS, ParsedFeedVersion,
    },
};

const ATOM_10_NAMESPACE: &str = "http://www.w3.org/2005/Atom";
const ATOM_03_NAMESPACE: &str = "http://purl.org/atom/ns#";
const RDF_NAMESPACE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RSS_10_NAMESPACE: &str = "http://purl.org/rss/1.0/";

pub(crate) struct PreflightedXml {
    pub(crate) parser_bytes: Vec<u8>,
    pub(crate) version: ParsedFeedVersion,
    pub(crate) feed_base: String,
    pub(crate) feed_link: Option<String>,
    pub(crate) entries: Vec<EntryPreflight>,
}

pub(crate) struct EntryPreflight {
    pub(crate) effective_base: String,
    pub(crate) raw_link: Option<String>,
    pub(crate) enclosure_urls: Vec<String>,
    pub(crate) summary_base: Option<String>,
    pub(crate) content_bases: Vec<String>,
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
    let mut entries = Vec::<EntryPreflight>::new();
    let mut base_stack = Vec::<String>::new();
    let mut entry_stack = Vec::<(usize, usize)>::new();
    let mut link_capture: Option<(usize, Option<usize>, String)> = None;

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
                let effective_base = effective_base(name, &attributes, parent_base)?;
                let local = local_name(name);
                if depth == 0 || local == "channel" {
                    feed_base.clone_from(&effective_base);
                }
                let current_entry = entry_stack.last().map(|(_, index)| *index);
                if matches!(local, "entry" | "item") {
                    let index = entries.len();
                    entries.push(EntryPreflight {
                        effective_base: effective_base.clone(),
                        raw_link: None,
                        enclosure_urls: Vec::new(),
                        summary_base: None,
                        content_bases: Vec::new(),
                    });
                    entry_stack.push((depth + 1, index));
                } else {
                    record_entry_element(
                        local,
                        &attributes,
                        &effective_base,
                        current_entry,
                        &mut entries,
                        &mut feed_link,
                        &mut link_capture,
                        depth + 1,
                    );
                }
                base_stack.push(effective_base);
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
                let effective_base = effective_base(name, &attributes, parent_base)?;
                let local = local_name(name);
                if depth == 0 || local == "channel" {
                    feed_base.clone_from(&effective_base);
                }
                let current_entry = entry_stack.last().map(|(_, index)| *index);
                if matches!(local, "entry" | "item") {
                    entries.push(EntryPreflight {
                        effective_base,
                        raw_link: None,
                        enclosure_urls: Vec::new(),
                        summary_base: None,
                        content_bases: Vec::new(),
                    });
                } else {
                    record_entry_element(
                        local,
                        &attributes,
                        &effective_base,
                        current_entry,
                        &mut entries,
                        &mut feed_link,
                        &mut None,
                        depth + 1,
                    );
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
                    && let Some((capture_depth, entry_index, value)) = link_capture.take()
                {
                    if capture_depth != depth {
                        return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
                    }
                    record_link(value.trim(), entry_index, &mut entries, &mut feed_link);
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
                if depth == 0 {
                    root_closed = true;
                }
            }
            Event::Text(text) => {
                let bytes: &[u8] = text.as_ref();
                if depth == 0 && !bytes.iter().all(u8::is_ascii_whitespace) {
                    return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
                }
                if let Some((_, _, value)) = link_capture.as_mut() {
                    value.push_str(
                        &text
                            .xml10_content()
                            .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedXml))?,
                    );
                }
            }
            Event::GeneralRef(reference) => {
                let character = validate_reference(&reference)?;
                if let Some((_, _, value)) = link_capture.as_mut() {
                    value.push(character);
                }
            }
            Event::DocType(_) => {
                return Err(FeedParseError::new(FeedParseErrorKind::DoctypeForbidden));
            }
            Event::Decl(_) => {
                return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
            }
            Event::Eof => break,
            Event::CData(_) | Event::Comment(_) | Event::PI(_) => {}
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
        entries,
    })
}

#[allow(clippy::too_many_arguments)]
fn record_entry_element(
    local: &str,
    attributes: &ElementAttributes,
    effective_base: &str,
    current_entry: Option<usize>,
    entries: &mut [EntryPreflight],
    feed_link: &mut Option<String>,
    link_capture: &mut Option<(usize, Option<usize>, String)>,
    element_depth: usize,
) {
    if local == "content"
        && let Some(index) = current_entry
    {
        entries[index].content_bases.push(effective_base.to_owned());
    } else if matches!(local, "summary" | "description")
        && let Some(index) = current_entry
        && entries[index].summary_base.is_none()
    {
        entries[index].summary_base = Some(effective_base.to_owned());
    }

    if local == "link" {
        if let Some(href) = attributes.get("href") {
            if attributes.get("rel") == Some("enclosure") {
                if let Some(index) = current_entry {
                    entries[index].enclosure_urls.push(href.to_owned());
                }
            } else if attributes.get("rel").is_none_or(|rel| rel == "alternate") {
                record_link(href, current_entry, entries, feed_link);
            }
        } else {
            *link_capture = Some((element_depth, current_entry, String::new()));
        }
    }
    if local == "enclosure"
        && let Some(index) = current_entry
        && let Some(url) = attributes.get("url")
    {
        entries[index].enclosure_urls.push(url.to_owned());
    }
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
    name: &str,
    attributes: &ElementAttributes,
    parent: &str,
) -> Result<String, FeedParseError> {
    let Some(explicit) = attributes.get("xml:base") else {
        return Ok(parent.to_owned());
    };
    if matches!(local_name(name), "link" | "enclosure") {
        return Err(FeedParseError::new(FeedParseErrorKind::InvalidUrl));
    }
    validate_base(explicit, parent)
}

fn validate_base(raw: &str, parent: &str) -> Result<String, FeedParseError> {
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
    Ok(normalized)
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
}
