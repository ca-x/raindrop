use std::{error::Error, fmt};

use super::sanitize::extract_rendered_text;

pub(crate) const MAX_ENTRY_SEARCH_TEXT_BYTES: usize = 60 * 1024;
#[allow(dead_code)] // Used by the Reader query slice immediately after projection delivery.
pub(crate) const MAX_SEARCH_QUERY_BYTES: usize = 128;
#[allow(dead_code)] // Used by the Reader query slice immediately after projection delivery.
pub(crate) const MAX_SEARCH_TERMS: usize = 8;

#[allow(dead_code)] // Used by the Reader query slice immediately after projection delivery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NormalizedSearch {
    terms: Vec<String>,
    canonical: String,
}

#[allow(dead_code)] // Used by the Reader query slice immediately after projection delivery.
impl NormalizedSearch {
    #[must_use]
    pub(crate) fn terms(&self) -> &[String] {
        &self.terms
    }

    #[must_use]
    pub(crate) fn canonical(&self) -> &str {
        &self.canonical
    }
}

#[allow(dead_code)] // Used by the Reader query slice immediately after projection delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SearchQueryError {
    Empty,
    TooLong,
    TooManyTerms,
}

impl fmt::Display for SearchQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "search query is empty",
            Self::TooLong => "search query is too long",
            Self::TooManyTerms => "search query has too many terms",
        })
    }
}

impl Error for SearchQueryError {}

#[must_use]
pub(crate) fn build_entry_search_text(
    title: Option<&str>,
    author: Option<&str>,
    summary: Option<&str>,
    content_html: &str,
) -> String {
    let rendered_content = extract_rendered_text(content_html);
    normalize_fields(
        [title, author, summary, Some(rendered_content.as_str())],
        MAX_ENTRY_SEARCH_TEXT_BYTES,
    )
}

#[allow(dead_code)] // Used by the Reader query slice immediately after projection delivery.
pub(crate) fn normalize_search_query(raw: &str) -> Result<NormalizedSearch, SearchQueryError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(SearchQueryError::Empty);
    }
    if trimmed.len() > MAX_SEARCH_QUERY_BYTES {
        return Err(SearchQueryError::TooLong);
    }

    let normalized = normalize_fields([Some(trimmed)], usize::MAX);
    let mut terms = Vec::new();
    for term in normalized.split(' ') {
        if terms.iter().any(|existing| existing == term) {
            continue;
        }
        if terms.len() == MAX_SEARCH_TERMS {
            return Err(SearchQueryError::TooManyTerms);
        }
        terms.push(term.to_owned());
    }
    if terms.is_empty() {
        return Err(SearchQueryError::Empty);
    }
    let canonical = terms.join(" ");
    Ok(NormalizedSearch { terms, canonical })
}

fn normalize_fields<const N: usize>(fields: [Option<&str>; N], max_bytes: usize) -> String {
    let mut output = String::new();
    let mut pending_space = false;
    for field in fields.into_iter().flatten() {
        if !output.is_empty() {
            pending_space = true;
        }
        if !append_normalized(&mut output, field, &mut pending_space, max_bytes) {
            break;
        }
    }
    output
}

fn append_normalized(
    output: &mut String,
    input: &str,
    pending_space: &mut bool,
    max_bytes: usize,
) -> bool {
    for character in input.chars().flat_map(char::to_lowercase) {
        if character.is_whitespace() {
            if !output.is_empty() {
                *pending_space = true;
            }
            continue;
        }
        let separator_bytes = usize::from(*pending_space && !output.is_empty());
        let required = separator_bytes.saturating_add(character.len_utf8());
        if output.len().saturating_add(required) > max_bytes {
            return false;
        }
        if separator_bytes == 1 {
            output.push(' ');
        }
        output.push(character);
        *pending_space = false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_search_text_uses_rendered_unicode_text_and_field_priority() {
        let text = build_entry_search_text(
            Some("  RUST\nNews "),
            Some("Alice"),
            Some("AT&amp;T summary"),
            "<p>ΓΕΙΑ&nbsp;κόσμε</p><p>100% _ literal</p>",
        );
        assert_eq!(
            text,
            "rust news alice at&amp;t summary γεια κόσμε 100% _ literal"
        );
    }

    #[test]
    fn entry_search_text_truncates_at_utf8_boundary() {
        let content = format!("<p>{}</p>", "界".repeat(MAX_ENTRY_SEARCH_TEXT_BYTES));
        let text = build_entry_search_text(Some("priority"), None, None, &content);
        assert!(text.starts_with("priority "));
        assert!(text.len() <= MAX_ENTRY_SEARCH_TEXT_BYTES);
        assert!(text.is_char_boundary(text.len()));
        assert!(!text.ends_with(' '));
    }

    #[test]
    fn search_query_normalizes_deduplicates_and_preserves_literals() {
        let search = normalize_search_query("  RUST\tRust  %_  ΓΕΙΑ ").unwrap();
        assert_eq!(search.terms(), ["rust", "%_", "γεια"]);
        assert_eq!(search.canonical(), "rust %_ γεια");
    }

    #[test]
    fn search_query_enforces_byte_and_term_bounds() {
        assert_eq!(normalize_search_query(" \n "), Err(SearchQueryError::Empty));
        assert_eq!(
            normalize_search_query(&"x".repeat(MAX_SEARCH_QUERY_BYTES + 1)),
            Err(SearchQueryError::TooLong)
        );
        assert_eq!(
            normalize_search_query("one two three four five six seven eight nine"),
            Err(SearchQueryError::TooManyTerms)
        );
        assert!(normalize_search_query("one two three four five six seven eight").is_ok());
    }
}
