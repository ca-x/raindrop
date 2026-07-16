use std::cell::RefCell;

use html5ever::{
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, StartTag, TagToken, Token, TokenSink, TokenSinkResult, Tokenizer,
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
