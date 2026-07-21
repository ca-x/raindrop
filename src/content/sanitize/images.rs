use std::cell::RefCell;

use html5ever::{
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, EndTag, StartTag, TagToken, Token, TokenSink, TokenSinkResult, Tokenizer,
        TokenizerOpts,
    },
};

use super::SanitizeError;

const MAX_ALT_BYTES: usize = 4 * 1024;
const MAX_DIMENSION: u32 = 16_384;

pub(super) struct ExtractedImage {
    pub(super) image_index: u32,
    pub(super) source_url: Option<String>,
    pub(super) alt: Option<String>,
    pub(super) width: Option<u32>,
    pub(super) height: Option<u32>,
}

struct ImageSink {
    images: RefCell<Vec<ExtractedImage>>,
}

impl TokenSink for ImageSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
        if let TagToken(tag) = token
            && tag.kind == StartTag
            && tag.name.as_ref() == "img"
        {
            let mut source_url = None;
            let mut alt = None;
            let mut width = None;
            let mut height = None;
            for attribute in tag.attrs {
                match attribute.name.local.as_ref() {
                    "src" => source_url = Some(attribute.value.to_string()),
                    "alt" => alt = Some(attribute.value.to_string()),
                    "width" => width = attribute.value.parse().ok(),
                    "height" => height = attribute.value.parse().ok(),
                    _ => {}
                }
            }
            let mut images = self.images.borrow_mut();
            let image_index = u32::try_from(images.len()).unwrap_or(u32::MAX);
            images.push(ExtractedImage {
                image_index,
                source_url,
                alt,
                width,
                height,
            });
        }
        TokenSinkResult::Continue
    }
}

pub(super) fn extract_images(html: &str) -> Vec<ExtractedImage> {
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let tokenizer = Tokenizer::new(
        ImageSink {
            images: RefCell::new(Vec::new()),
        },
        TokenizerOpts::default(),
    );
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.images.into_inner()
}

struct TextSink {
    text: RefCell<String>,
}

impl TokenSink for TextSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
        if let Token::CharacterTokens(characters) = token {
            self.text.borrow_mut().push_str(&characters);
        }
        TokenSinkResult::Continue
    }
}

pub(super) fn extract_text(html: &str) -> String {
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let tokenizer = Tokenizer::new(
        TextSink {
            text: RefCell::new(String::new()),
        },
        TokenizerOpts::default(),
    );
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.text.into_inner()
}

struct SearchTextSink {
    text: RefCell<String>,
}

impl TokenSink for SearchTextSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
        match token {
            Token::CharacterTokens(characters) => self.text.borrow_mut().push_str(&characters),
            TagToken(tag) if is_search_text_boundary(tag.name.as_ref()) => {
                self.text.borrow_mut().push(' ');
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

pub(super) fn extract_search_text(html: &str) -> String {
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let tokenizer = Tokenizer::new(
        SearchTextSink {
            text: RefCell::new(String::new()),
        },
        TokenizerOpts::default(),
    );
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.text.into_inner()
}

fn is_search_text_boundary(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "br"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "figcaption"
            | "figure"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "tr"
            | "ul"
    )
}

struct TranslationSegmentSink {
    current: RefCell<String>,
    segments: RefCell<Vec<String>>,
}

impl TranslationSegmentSink {
    fn flush(&self) {
        let normalized = normalize_segment(&self.current.borrow());
        self.current.borrow_mut().clear();
        if !normalized.is_empty() {
            self.segments.borrow_mut().push(normalized);
        }
    }
}

impl TokenSink for TranslationSegmentSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
        match token {
            Token::CharacterTokens(characters) => self.current.borrow_mut().push_str(&characters),
            TagToken(tag)
                if matches!(tag.kind, StartTag | EndTag)
                    && is_translation_segment_boundary(tag.name.as_ref()) =>
            {
                self.flush();
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

pub(super) fn extract_translation_segments(html: &str) -> Vec<String> {
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let tokenizer = Tokenizer::new(
        TranslationSegmentSink {
            current: RefCell::new(String::new()),
            segments: RefCell::new(Vec::new()),
        },
        TokenizerOpts::default(),
    );
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.flush();
    tokenizer.sink.segments.into_inner()
}

fn normalize_segment(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push(character);
            pending_space = false;
        }
    }
    normalized
}

fn is_translation_segment_boundary(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "blockquote"
            | "dd"
            | "div"
            | "dt"
            | "figcaption"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "li"
            | "p"
            | "pre"
            | "td"
            | "th"
    )
}

struct ValidationSink {
    error: RefCell<Option<SanitizeError>>,
}

impl TokenSink for ValidationSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<Self::Handle> {
        if self.error.borrow().is_some() {
            return TokenSinkResult::Continue;
        }
        if let TagToken(tag) = token
            && tag.kind == StartTag
            && tag.name.as_ref() == "img"
        {
            for attribute in tag.attrs {
                let error =
                    match attribute.name.local.as_ref() {
                        "alt" if attribute.value.len() > MAX_ALT_BYTES => {
                            Some(SanitizeError::ImageAltTooLong {
                                bytes: attribute.value.len(),
                            })
                        }
                        "width" | "height"
                            if attribute.value.parse::<u32>().ok().is_none_or(|dimension| {
                                !(1..=MAX_DIMENSION).contains(&dimension)
                            }) =>
                        {
                            Some(SanitizeError::ImageDimensionInvalid)
                        }
                        _ => None,
                    };
                if error.is_some() {
                    *self.error.borrow_mut() = error;
                    break;
                }
            }
        }
        TokenSinkResult::Continue
    }
}

pub(super) fn validate_image_metadata(html: &str) -> Result<(), SanitizeError> {
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(html));
    let tokenizer = Tokenizer::new(
        ValidationSink {
            error: RefCell::new(None),
        },
        TokenizerOpts::default(),
    );
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.error.into_inner().map_or(Ok(()), Err)
}
