use std::mem::size_of;

use serde::Deserialize;
use serde_json::Value;

use super::types::{
    FeedParseError, FeedParseErrorKind, MAX_CONTENT_BYTES, MAX_DEPTH, MAX_ENCLOSURES, MAX_ENTRIES,
    MAX_TITLE_BYTES, ProjectedInheritance,
};

pub(crate) struct PreflightedJson {
    pub(crate) parser_bytes: Vec<u8>,
    pub(crate) projected_inheritance_bytes: usize,
}

pub(crate) fn preflight(input: &str) -> Result<PreflightedJson, FeedParseError> {
    check_raw_depth(input)?;
    let mut deserializer = serde_json::Deserializer::from_str(input);
    deserializer.disable_recursion_limit();
    let value = Value::deserialize(&mut deserializer)
        .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
    deserializer
        .end()
        .map_err(|_| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
    check_depth(&value, 1)?;
    let object = value
        .as_object()
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MimeMismatch))?;
    let version = match object.get("version") {
        Some(Value::String(version)) => version.as_str(),
        Some(_) => return Err(FeedParseError::new(FeedParseErrorKind::UnsupportedVersion)),
        None if looks_like_json_feed(object) => {
            return Err(FeedParseError::new(FeedParseErrorKind::UnsupportedVersion));
        }
        None => return Err(FeedParseError::new(FeedParseErrorKind::MimeMismatch)),
    };
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
    if let Some(author) = object.get("author") {
        check_author(author)?;
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
    let projected_inheritance_bytes = project_inheritance(object, items)?.bytes();
    let mut remainder = value;
    project_for_parser(&mut remainder);
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
    Ok(PreflightedJson {
        parser_bytes,
        projected_inheritance_bytes,
    })
}

fn project_inheritance(
    feed: &serde_json::Map<String, Value>,
    items: &[Value],
) -> Result<ProjectedInheritance, FeedParseError> {
    let authors = json_feed_authors(feed)?;
    let author_clone_bytes = if authors.count == 0 {
        0
    } else {
        let mut vector = ProjectedInheritance::default();
        vector.add_product(authors.count, size_of::<feedparser_rs::Person>())?;
        vector.add(authors.payload_bytes)?;

        let mut per_item = ProjectedInheritance::default();
        per_item.add(vector.bytes())?;
        if authors.first_has_name {
            per_item.add(size_of::<feedparser_rs::types::SmallString>())?;
            per_item.add(authors.first_name_bytes)?;
        }
        per_item.add(size_of::<feedparser_rs::Person>())?;
        per_item.add(authors.first_payload_bytes)?;
        per_item.bytes()
    };
    let feed_language = feed
        .get("language")
        .and_then(Value::as_str)
        .filter(|language| !language.is_empty());
    let language_clone_bytes = if let Some(language) = feed_language {
        let mut clone = ProjectedInheritance::default();
        clone.add(size_of::<feedparser_rs::types::SmallString>())?;
        clone.add(language.len())?;
        Some(clone.bytes())
    } else {
        None
    };

    let mut projected = ProjectedInheritance::default();
    for item in items {
        let item = item.as_object().expect("item objects were validated");
        if author_clone_bytes != 0 && json_item_inherits_authors(item) {
            projected.add(author_clone_bytes)?;
        }
        if let Some(clone_bytes) = language_clone_bytes {
            let item_language = item
                .get("language")
                .and_then(Value::as_str)
                .filter(|language| !language.is_empty());
            if item_language.is_none() {
                let destinations = 1 + ["title", "content_html", "content_text", "summary"]
                    .into_iter()
                    .filter(|field| item.contains_key(*field))
                    .count();
                projected.add_product(destinations, clone_bytes)?;
            }
        }
    }
    Ok(projected)
}

#[derive(Default)]
struct JsonFeedAuthors {
    count: usize,
    payload_bytes: usize,
    first_payload_bytes: usize,
    first_name_bytes: usize,
    first_has_name: bool,
}

fn json_feed_authors(
    feed: &serde_json::Map<String, Value>,
) -> Result<JsonFeedAuthors, FeedParseError> {
    let authors = if let Some(authors) = feed.get("authors") {
        authors
            .as_array()
            .expect("feed authors collection was validated")
            .iter()
            .collect::<Vec<_>>()
    } else {
        feed.get("author").into_iter().collect::<Vec<_>>()
    };
    let mut stats = JsonFeedAuthors::default();
    for author in authors {
        let author = author.as_object().expect("author objects were validated");
        let name = author.get("name").and_then(Value::as_str);
        let mut payload = ProjectedInheritance::default();
        for field in ["name", "url", "avatar"] {
            if let Some(value) = author.get(field).and_then(Value::as_str) {
                payload.add(value.len())?;
            }
        }
        if stats.count == 0 {
            stats.first_payload_bytes = payload.bytes();
            stats.first_name_bytes = name.map_or(0, str::len);
            stats.first_has_name = name.is_some();
        }
        stats.count = stats.count.checked_add(1).ok_or_else(|| {
            FeedParseError::new(FeedParseErrorKind::ProjectedInheritanceTooLarge)
                .with_bytes(usize::MAX)
        })?;
        stats.payload_bytes = stats
            .payload_bytes
            .checked_add(payload.bytes())
            .ok_or_else(|| {
                FeedParseError::new(FeedParseErrorKind::ProjectedInheritanceTooLarge)
                    .with_bytes(usize::MAX)
            })?;
    }
    Ok(stats)
}

fn json_item_inherits_authors(item: &serde_json::Map<String, Value>) -> bool {
    if let Some(authors) = item.get("authors") {
        return authors
            .as_array()
            .expect("item authors collection was validated")
            .is_empty();
    }
    !item.contains_key("author")
}

fn project_for_parser(value: &mut Value) {
    let object = value
        .as_object_mut()
        .expect("top-level object was validated");
    object.retain(|key, value| {
        matches!(
            key.as_str(),
            "version"
                | "title"
                | "home_page_url"
                | "feed_url"
                | "description"
                | "user_comment"
                | "next_url"
                | "icon"
                | "favicon"
                | "authors"
                | "author"
                | "language"
                | "items"
                | "hubs"
        ) || (key == "expired" && value.is_boolean())
    });
    project_authors(object.get_mut("authors"));
    project_author(object.get_mut("author"));
    if let Some(hubs) = object.get_mut("hubs").and_then(Value::as_array_mut) {
        for hub in hubs {
            hub.as_object_mut()
                .expect("hub objects were validated")
                .retain(|key, _| matches!(key.as_str(), "type" | "url"));
        }
    }
    if let Some(items) = object.get_mut("items").and_then(Value::as_array_mut) {
        for item in items {
            let item = item.as_object_mut().expect("item objects were validated");
            item.retain(|key, _| {
                matches!(
                    key.as_str(),
                    "id" | "url"
                        | "external_url"
                        | "title"
                        | "content_html"
                        | "content_text"
                        | "summary"
                        | "image"
                        | "banner_image"
                        | "date_published"
                        | "date_modified"
                        | "authors"
                        | "author"
                        | "tags"
                        | "language"
                        | "attachments"
                )
            });
            project_authors(item.get_mut("authors"));
            project_author(item.get_mut("author"));
            if let Some(attachments) = item.get_mut("attachments").and_then(Value::as_array_mut) {
                for attachment in attachments {
                    attachment
                        .as_object_mut()
                        .expect("attachment objects were validated")
                        .retain(|key, _| {
                            matches!(
                                key.as_str(),
                                "url"
                                    | "mime_type"
                                    | "title"
                                    | "size_in_bytes"
                                    | "duration_in_seconds"
                            )
                        });
                }
            }
        }
    }
}

fn project_authors(value: Option<&mut Value>) {
    if let Some(authors) = value.and_then(Value::as_array_mut) {
        for author in authors {
            project_author(Some(author));
        }
    }
}

fn project_author(value: Option<&mut Value>) {
    if let Some(author) = value.and_then(Value::as_object_mut) {
        author.retain(|key, _| matches!(key.as_str(), "name" | "url" | "avatar"));
    }
}

fn looks_like_json_feed(object: &serde_json::Map<String, Value>) -> bool {
    [
        "title",
        "home_page_url",
        "feed_url",
        "description",
        "user_comment",
        "next_url",
        "icon",
        "favicon",
        "authors",
        "author",
        "language",
        "items",
        "hubs",
    ]
    .iter()
    .any(|field| object.contains_key(*field))
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
    if let Some(author) = object.get("author") {
        check_author(author)?;
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
            for field in ["url", "mime_type"] {
                check_required_string(
                    attachment.get(field),
                    MAX_CONTENT_BYTES,
                    FeedParseErrorKind::ContentTooLong,
                )?;
            }
            check_optional_string(
                attachment.get("title"),
                MAX_CONTENT_BYTES,
                FeedParseErrorKind::ContentTooLong,
            )?;
            for field in ["duration_in_seconds", "size_in_bytes"] {
                check_optional_nonnegative_integer(attachment.get(field))?;
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

fn check_required_string(
    value: Option<&Value>,
    maximum: usize,
    kind: FeedParseErrorKind,
) -> Result<(), FeedParseError> {
    let string = value
        .and_then(Value::as_str)
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedJson))?;
    if string.len() > maximum {
        Err(FeedParseError::new(kind).with_bytes(string.len()))
    } else {
        Ok(())
    }
}

fn check_optional_nonnegative_integer(value: Option<&Value>) -> Result<(), FeedParseError> {
    let Some(value) = value else {
        return Ok(());
    };
    value
        .as_u64()
        .map(|_| ())
        .ok_or_else(|| FeedParseError::new(FeedParseErrorKind::MalformedJson))
}

fn check_depth(value: &Value, depth: usize) -> Result<(), FeedParseError> {
    match value {
        Value::Array(values) => {
            if depth > MAX_DEPTH {
                return Err(FeedParseError::new(FeedParseErrorKind::DepthLimit).with_count(depth));
            }
            for value in values {
                check_depth(value, depth + 1)?;
            }
        }
        Value::Object(values) => {
            if depth > MAX_DEPTH {
                return Err(FeedParseError::new(FeedParseErrorKind::DepthLimit).with_count(depth));
            }
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

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use serde_json::json;

    use super::*;

    #[test]
    fn projection_matches_json_author_and_language_precedence() {
        let value = json!({
            "authors":[{"name":"n"}],
            "language":"ll",
            "items":[
                {"id":"1","content_text":"x"},
                {"id":"2","authors":[],"author":{"name":"own"},"language":"","title":"x"},
                {"id":"3","authors":[{}],"language":"own"},
                {"id":"4","author":{},"language":"own"}
            ]
        });
        let feed = value.as_object().expect("object");
        let items = feed.get("items").and_then(Value::as_array).expect("items");
        let author_clone = 2 * size_of::<feedparser_rs::Person>()
            + size_of::<feedparser_rs::types::SmallString>()
            + 3;
        let language_clone = size_of::<feedparser_rs::types::SmallString>() + 2;
        assert_eq!(
            project_inheritance(feed, items)
                .expect("projection")
                .bytes(),
            2 * author_clone + 4 * language_clone
        );
    }
}
