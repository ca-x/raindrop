use quick_xml::{Reader, events::Event};

use super::types::{FeedParseError, FeedParseErrorKind, MAX_EVENTS};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BodyFormat {
    Xml,
    Json,
}

#[derive(Clone, Debug)]
pub(crate) struct MimeDecision {
    pub(crate) expected: Option<BodyFormat>,
    pub(crate) charset: Option<String>,
}

pub(crate) fn classify(content_type: Option<&str>) -> Result<MimeDecision, FeedParseError> {
    let Some(raw) = content_type else {
        return Ok(MimeDecision {
            expected: None,
            charset: None,
        });
    };
    let parsed: mime::Mime = raw
        .parse()
        .map_err(|_| FeedParseError::new(FeedParseErrorKind::UnsupportedContentType))?;
    let charsets: Vec<_> = parsed
        .params()
        .filter(|(name, _)| *name == mime::CHARSET)
        .map(|(_, value)| value.as_str().to_owned())
        .collect();
    if charsets.len() > 1 {
        return Err(
            FeedParseError::new(FeedParseErrorKind::UnsupportedContentType)
                .with_count(charsets.len()),
        );
    }
    let essence = parsed.essence_str();
    let expected = match essence {
        "application/rss+xml"
        | "application/atom+xml"
        | "application/rdf+xml"
        | "application/xml"
        | "text/xml" => Some(BodyFormat::Xml),
        "application/feed+json" | "application/json" => Some(BodyFormat::Json),
        "text/plain" | "text/html" | "application/octet-stream" => None,
        _ => {
            return Err(FeedParseError::new(
                FeedParseErrorKind::UnsupportedContentType,
            ));
        }
    };
    Ok(MimeDecision {
        expected,
        charset: charsets.into_iter().next(),
    })
}

pub(crate) fn sniff(decoded: &str) -> Option<BodyFormat> {
    let leading = decoded.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    if leading.starts_with('{') {
        return Some(BodyFormat::Json);
    }

    let xml = decoded.strip_prefix('\u{feff}').unwrap_or(decoded);
    let mut reader = Reader::from_str(xml);
    reader.config_mut().enable_all_checks(true);
    let mut events = 0_usize;
    loop {
        let event = reader.read_event().ok()?;
        if !matches!(event, Event::Eof) {
            events += 1;
            if events > MAX_EVENTS {
                return None;
            }
        }
        match event {
            Event::Start(element) | Event::Empty(element) => {
                return matches!(element.local_name().as_ref(), b"rss" | b"feed" | b"RDF")
                    .then_some(BodyFormat::Xml);
            }
            Event::Text(text) => {
                let bytes: &[u8] = text.as_ref();
                if !bytes.iter().all(u8::is_ascii_whitespace) {
                    return None;
                }
            }
            Event::Decl(_)
            | Event::Comment(_)
            | Event::PI(_)
            | Event::DocType(_)
            | Event::CData(_)
            | Event::GeneralRef(_) => {}
            Event::Eof | Event::End(_) => return None,
        }
    }
}
