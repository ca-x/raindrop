//! Conservative, model-independent text budgets. Configured limits apply to each request.

/// Application validation ceiling; actual requests also obey the user and Provider limits.
pub const MAX_OUTPUT_TOKENS: u32 = 1_048_576;

/// Maximum number of source chunks in one article translation.
pub const MAX_TRANSLATION_CHUNKS: u32 = 128;
/// Space for JSON, translated titles and model reasoning in each request.
pub const TRANSLATION_OVERHEAD_TOKENS: u32 = 1_024;

/// Estimate translation output with extra room for scripts that use more tokens per character.
#[must_use]
pub fn translation_tokens(text: &str, target_locale: &str) -> u32 {
    let units = text_units(text);
    let expansion = if target_locale.starts_with("zh")
        || target_locale.starts_with("ja")
        || target_locale.starts_with("ko")
    {
        2
    } else {
        1
    };
    units
        .saturating_mul(expansion)
        .saturating_add(TRANSLATION_OVERHEAD_TOKENS)
}

/// Split at bounded UTF-8 boundaries, preferring paragraphs, sentences and spaces.
/// Joining the returned slices reproduces the input byte for byte.
#[must_use]
pub fn split_text(text: &str, maximum_units: u32) -> Vec<&str> {
    let maximum_units = maximum_units.max(4);
    let mut chunks = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut units = 0_u32;
        let mut end = rest.len();
        let mut boundary = None;
        for (index, character) in rest.char_indices() {
            let weight = character_units(character);
            if units.saturating_add(weight) > maximum_units {
                end = boundary.filter(|&value| value > index / 2).unwrap_or(index);
                break;
            }
            units += weight;
            if character.is_whitespace()
                || matches!(character, '.' | '!' | '?' | '。' | '！' | '？')
            {
                boundary = Some(index + character.len_utf8());
            }
        }
        chunks.push(&rest[..end]);
        rest = &rest[end..];
    }
    chunks
}

/// A conservative text-size score, independent of a vendor's tokenizer.
#[must_use]
pub fn text_units(text: &str) -> u32 {
    text.chars().fold(0_u32, |sum, character| {
        sum.saturating_add(character_units(character))
    })
}

fn character_units(character: char) -> u32 {
    if character.is_ascii() { 1 } else { 3 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_preserve_all_unicode_and_whitespace_at_the_limit() {
        for text in [
            " ",
            "abc\n\ndef  gh\tijk",
            "中文🙂é。段落\n\n日本語 👩‍💻",
            "word",
        ]
        .map(str::to_owned)
        .into_iter()
        .chain(["x".repeat(50_000)])
        {
            for limit in [4, 9, 128, 3072] {
                let parts = split_text(&text, limit);
                assert_eq!(parts.concat(), text);
                assert!(
                    parts
                        .iter()
                        .all(|part| !part.is_empty() && text_units(part) <= limit)
                );
            }
        }
    }
}
