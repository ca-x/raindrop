use super::types::{FeedParseError, FeedParseErrorKind};

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
        Some(BodyFormat::Json)
    } else if leading.starts_with("<?xml")
        || leading.starts_with("<rss")
        || leading.starts_with("<feed")
        || leading.starts_with("<rdf:RDF")
        || leading.starts_with("<RDF")
    {
        Some(BodyFormat::Xml)
    } else {
        None
    }
}
