const SOURCE_CONTEXT: &str = "raindrop.entry-source-content.v1";
const CONTENT_CONTEXT: &str = "raindrop.entry-content.v1";
const FRAME_HEADER: &[u8] = b"RDHC\0\x01";
const HTML_TAG: u8 = 1;

pub(crate) fn source_content_hash(html: &str) -> [u8; 32] {
    semantic_hash(SOURCE_CONTEXT, html)
}

pub(crate) fn content_hash(html: &str) -> [u8; 32] {
    semantic_hash(CONTENT_CONTEXT, html)
}

fn semantic_hash(context: &str, html: &str) -> [u8; 32] {
    let length = u32::try_from(html.len()).expect("sanitized HTML is bounded below u32::MAX");
    let mut frame = Vec::with_capacity(FRAME_HEADER.len() + 1 + 4 + html.len());
    frame.extend_from_slice(FRAME_HEADER);
    frame.push(HTML_TAG);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(html.as_bytes());
    blake3::derive_key(context, &frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_are_distinct_for_the_same_frame() {
        assert_ne!(source_content_hash("<p>x</p>"), content_hash("<p>x</p>"));
    }

    #[test]
    fn frame_has_the_frozen_independent_golden_digest() {
        assert_eq!(
            hex(&source_content_hash("<p>x</p>")),
            "9ba8a700419a7f3b654fd0f162e6fc4da7eaceeab30408c2700b542d7d6df872"
        );
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
