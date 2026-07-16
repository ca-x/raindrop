use serde_json::Value;

use super::types::{
    FeedParseError, FeedParseErrorKind, MAX_CONTENT_BYTES, MAX_DEPTH, MAX_ENCLOSURES, MAX_ENTRIES,
    MAX_TITLE_BYTES,
};

pub(crate) struct PreflightedJson {
    pub(crate) parser_bytes: Vec<u8>,
}

pub(crate) fn preflight(input: &str) -> Result<PreflightedJson, FeedParseError> {
    check_raw_depth(input)?;
    let value: Value = serde_json::from_str(input)
        .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
    check_depth(&value, 1)?;
    let object = value
        .as_object()
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MimeMismatch))?;
    let version = object
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MimeMismatch))?;
    if !matches!(
        version,
        "https://jsonfeed.org/version/1" | "https://jsonfeed.org/version/1.1"
    ) {
        return Err(FeedParseError::new(FeedParseErrorKind::UnsupportedVersion));
    }
    check_optional_string(
        object.get("title"),
        MAX_TITLE_BYTES,
        FeedParseErrorKind::TitleTooLong,
    )?;
    for field in [
        "home_page_url",
        "feed_url",
        "description",
        "user_comment",
        "next_url",
        "icon",
        "favicon",
        "language",
    ] {
        check_optional_string(
            object.get(field),
            MAX_CONTENT_BYTES,
            FeedParseErrorKind::ContentTooLong,
        )?;
    }
    check_collection(object.get("authors"), 256)?;
    check_collection(object.get("hubs"), 256)?;
    if let Some(authors) = object.get("authors").and_then(Value::as_array) {
        for author in authors {
            check_author(author)?;
        }
    }
    if let Some(hubs) = object.get("hubs").and_then(Value::as_array) {
        for hub in hubs {
            let hub = hub
                .as_object()
                .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
            for field in ["type", "url"] {
                check_optional_string(
                    hub.get(field),
                    MAX_CONTENT_BYTES,
                    FeedParseErrorKind::ContentTooLong,
                )?;
            }
        }
    }
    let items = object
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
    if items.len() > MAX_ENTRIES {
        return Err(FeedParseError::new(FeedParseErrorKind::TooManyEntries).with_count(items.len()));
    }
    for item in items {
        check_item(item)?;
    }
    let mut remainder = value;
    let object = remainder
        .as_object_mut()
        .expect("top-level object was validated");
    let version = object.remove("version").expect("version was validated");
    let remainder = serde_json::to_vec(&remainder)
        .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
    let version = serde_json::to_vec(&version)
        .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
    let mut parser_bytes = Vec::with_capacity(remainder.len() + version.len() + 11);
    parser_bytes.extend_from_slice(b"{\"version\":");
    parser_bytes.extend_from_slice(&version);
    if remainder.len() > 2 {
        parser_bytes.push(b',');
        parser_bytes.extend_from_slice(&remainder[1..]);
    } else {
        parser_bytes.push(b'}');
    }
    Ok(PreflightedJson { parser_bytes })
}

fn check_item(item: &Value) -> Result<(), FeedParseError> {
    let object = item
        .as_object()
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
    for field in [
        "id",
        "url",
        "external_url",
        "image",
        "banner_image",
        "date_published",
        "date_modified",
        "language",
    ] {
        check_optional_string(
            object.get(field),
            MAX_CONTENT_BYTES,
            FeedParseErrorKind::ContentTooLong,
        )?;
    }
    check_optional_string(
        object.get("title"),
        MAX_TITLE_BYTES,
        FeedParseErrorKind::TitleTooLong,
    )?;
    for field in ["content_html", "content_text", "summary"] {
        check_optional_string(
            object.get(field),
            MAX_CONTENT_BYTES,
            FeedParseErrorKind::ContentTooLong,
        )?;
    }
    check_collection(object.get("authors"), 256)?;
    check_collection(object.get("tags"), 256)?;
    if let Some(authors) = object.get("authors").and_then(Value::as_array) {
        for author in authors {
            check_author(author)?;
        }
    }
    if let Some(tags) = object.get("tags").and_then(Value::as_array) {
        for tag in tags {
            check_optional_string(
                Some(tag),
                MAX_CONTENT_BYTES,
                FeedParseErrorKind::ContentTooLong,
            )?;
        }
    }
    if let Some(attachments) = object.get("attachments") {
        let attachments = attachments
            .as_array()
            .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
        if attachments.len() > MAX_ENCLOSURES {
            return Err(FeedParseError::new(FeedParseErrorKind::TooManyEnclosures)
                .with_count(attachments.len()));
        }
        for attachment in attachments {
            let attachment = attachment
                .as_object()
                .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
            for field in [
                "url",
                "mime_type",
                "title",
                "duration_in_seconds",
                "size_in_bytes",
            ] {
                if let Some(value) = attachment.get(field)
                    && value.is_string()
                {
                    check_optional_string(
                        Some(value),
                        MAX_CONTENT_BYTES,
                        FeedParseErrorKind::ContentTooLong,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn check_author(author: &Value) -> Result<(), FeedParseError> {
    let author = author
        .as_object()
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
    for field in ["name", "url", "avatar"] {
        check_optional_string(
            author.get(field),
            MAX_CONTENT_BYTES,
            FeedParseErrorKind::ContentTooLong,
        )?;
    }
    Ok(())
}

fn check_collection(value: Option<&Value>, maximum: usize) -> Result<(), FeedParseError> {
    let Some(value) = value else {
        return Ok(());
    };
    let values = value
        .as_array()
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
    if values.len() > maximum {
        Err(FeedParseError::new(FeedParseErrorKind::ParserFailure).with_count(values.len()))
    } else {
        Ok(())
    }
}

fn check_optional_string(
    value: Option<&Value>,
    maximum: usize,
    kind: FeedParseErrorKind,
) -> Result<(), FeedParseError> {
    let Some(value) = value else {
        return Ok(());
    };
    let string = value
        .as_str()
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
    if string.len() > maximum {
        Err(FeedParseError::new(kind).with_bytes(string.len()))
    } else {
        Ok(())
    }
}

fn check_depth(value: &Value, depth: usize) -> Result<(), FeedParseError> {
    if depth > MAX_DEPTH {
        return Err(FeedParseError::new(FeedParseErrorKind::DepthLimit).with_count(depth));
    }
    match value {
        Value::Array(values) => {
            for value in values {
                check_depth(value, depth + 1)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                check_depth(value, depth + 1)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

fn check_raw_depth(input: &str) -> Result<(), FeedParseError> {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in input.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(
                        FeedParseError::new(FeedParseErrorKind::DepthLimit).with_count(depth)
                    );
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}
