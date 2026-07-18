use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};

use super::types::{FeedParseError, FeedParseErrorKind, MAX_DOCUMENT_BYTES};

pub(crate) struct DecodedDocument {
    pub(crate) utf8: String,
    pub(crate) original_encoding: String,
    pub(crate) source_document_hash: [u8; 32],
}

#[derive(Clone, Debug)]
pub(crate) struct XmlDeclaration {
    pub(crate) encoding: Option<String>,
    pub(crate) end: usize,
}

pub(crate) fn decode(
    raw: &[u8],
    http_charset: Option<&str>,
) -> Result<DecodedDocument, FeedParseError> {
    if raw.len() > MAX_DOCUMENT_BYTES {
        return Err(
            FeedParseError::new(FeedParseErrorKind::ConvertedTooLarge).with_bytes(raw.len())
        );
    }
    let source_document_hash = *blake3::hash(raw).as_bytes();
    let (bom_encoding, bom_len) = detect_bom(raw)?;
    let ascii_declaration = if bom_encoding.is_none() {
        parse_ascii_declaration(raw)?
    } else {
        None
    };

    let http_encoding = http_charset.map(resolve_encoding).transpose()?;
    let declaration_encoding = ascii_declaration
        .as_ref()
        .and_then(|declaration| declaration.encoding.as_deref())
        .map(resolve_encoding)
        .transpose()?;
    let selected = bom_encoding
        .or(http_encoding)
        .or(declaration_encoding)
        .unwrap_or(UTF_8);
    let bytes = &raw[bom_len..];
    let decoded = selected
        .decode_without_bom_handling_and_without_replacement(bytes)
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::DecodeFailed))?;
    if decoded.contains('\u{fffd}') {
        return Err(FeedParseError::new(FeedParseErrorKind::DecodeFailed));
    }
    if decoded.len() > MAX_DOCUMENT_BYTES {
        return Err(
            FeedParseError::new(FeedParseErrorKind::ConvertedTooLarge).with_bytes(decoded.len())
        );
    }
    let utf8 = decoded.into_owned();
    if let Some(declaration) = xml_declaration(&utf8)?
        && let Some(label) = declaration.encoding.as_deref()
    {
        let _ = resolve_encoding(label)?;
    }
    Ok(DecodedDocument {
        utf8,
        original_encoding: selected.name().to_ascii_lowercase(),
        source_document_hash,
    })
}

fn detect_bom(raw: &[u8]) -> Result<(Option<&'static Encoding>, usize), FeedParseError> {
    if raw.starts_with(&[0x00, 0x00, 0xfe, 0xff]) || raw.starts_with(&[0xff, 0xfe, 0x00, 0x00]) {
        return Err(FeedParseError::new(FeedParseErrorKind::UnsupportedCharset));
    }
    if raw.starts_with(&[0xef, 0xbb, 0xbf]) {
        Ok((Some(UTF_8), 3))
    } else if raw.starts_with(&[0xfe, 0xff]) {
        Ok((Some(UTF_16BE), 2))
    } else if raw.starts_with(&[0xff, 0xfe]) {
        Ok((Some(UTF_16LE), 2))
    } else {
        Ok((None, 0))
    }
}

fn resolve_encoding(label: &str) -> Result<&'static Encoding, FeedParseError> {
    let normalized = label.trim().to_ascii_lowercase();
    if normalized.starts_with("utf-32") || normalized == "utf32" {
        return Err(FeedParseError::new(FeedParseErrorKind::UnsupportedCharset));
    }
    Encoding::for_label(normalized.as_bytes())
        .filter(|encoding| encoding.name() != "replacement")
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::UnsupportedCharset))
}

fn parse_ascii_declaration(raw: &[u8]) -> Result<Option<XmlDeclaration>, FeedParseError> {
    if !raw.starts_with(b"<?xml") {
        return Ok(None);
    }
    let end = raw
        .windows(2)
        .position(|window| window == b"?>")
        .map(|position| position + 2)
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedXml))?;
    let declaration = std::str::from_utf8(&raw[..end])
        .ok()
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedXml))?;
    xml_declaration(declaration)
}

pub(crate) fn xml_declaration(input: &str) -> Result<Option<XmlDeclaration>, FeedParseError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    if !input.starts_with("<?xml") {
        return Ok(None);
    }
    let end = input
        .find("?>")
        .map(|position| position + 2)
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedXml))?;
    let body = input
        .get(5..end - 2)
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedXml))?;
    if body.is_empty() || !body.as_bytes()[0].is_ascii_whitespace() {
        return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
    }
    let attributes = parse_declaration_attributes(body)?;
    if attributes.first().map(|(name, _)| name.as_str()) != Some("version")
        || attributes.first().map(|(_, value)| value.as_str()) != Some("1.0")
        || attributes
            .iter()
            .any(|(name, _)| !matches!(name.as_str(), "version" | "encoding" | "standalone"))
    {
        return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
    }
    let encoding = attributes
        .iter()
        .find(|(name, _)| name == "encoding")
        .map(|(_, value)| value.clone());
    if let Some(label) = encoding.as_deref()
        && !valid_encoding_name(label)
    {
        return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
    }
    if let Some(standalone) = attributes
        .iter()
        .find(|(name, _)| name == "standalone")
        .map(|(_, value)| value.as_str())
        && !matches!(standalone, "yes" | "no")
    {
        return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
    }
    Ok(Some(XmlDeclaration { encoding, end }))
}

fn parse_declaration_attributes(body: &str) -> Result<Vec<(String, String)>, FeedParseError> {
    let bytes = body.as_bytes();
    let mut attributes = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor == bytes.len() {
            break;
        }
        let name_start = cursor;
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric()
                || matches!(bytes[cursor], b'_' | b':' | b'-'))
        {
            cursor += 1;
        }
        if cursor == name_start {
            return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
        }
        let name = &body[name_start..cursor];
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let quote = *bytes
            .get(cursor)
            .filter(|quote| matches!(quote, b'\'' | b'"'))
            .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedXml))?;
        cursor += 1;
        let value_start = cursor;
        while cursor < bytes.len() && bytes[cursor] != quote {
            cursor += 1;
        }
        if cursor == bytes.len() {
            return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
        }
        let value = &body[value_start..cursor];
        cursor += 1;
        if attributes.iter().any(|(existing, _)| existing == name) {
            return Err(FeedParseError::new(FeedParseErrorKind::MalformedXml));
        }
        attributes.push((name.to_owned(), value.to_owned()));
    }
    Ok(attributes)
}

fn valid_encoding_name(label: &str) -> bool {
    let mut bytes = label.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
