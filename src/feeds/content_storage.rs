use std::fmt;

use serde::{Deserialize, Serialize};

use crate::content::sanitize::{StoredContentValidationError, validate_stored_content};
use crate::content::{InertImage, SanitizedContent};

const STORAGE_PREFIX: &str = "rdsc:v1:";
const MAX_ENVELOPE_BYTES: usize = 4 * 1024 * 1024;
const MAX_HTML_BYTES: usize = 1024 * 1024;
const MAX_IMAGES: usize = 256;
const MAX_IMAGE_METADATA_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EntryContentError {
    #[error("entry content envelope is too large")]
    EnvelopeTooLarge,
    #[error("entry content envelope version is unsupported")]
    UnsupportedVersion,
    #[error("entry content envelope is malformed")]
    Malformed,
    #[error("entry content HTML is too large")]
    HtmlTooLarge,
    #[error("entry content has too many inert images")]
    TooManyImages,
    #[error("entry content inert image metadata is too large")]
    ImageMetadataTooLarge,
    #[error("entry content envelope is not canonical")]
    NonCanonical,
    #[error("entry content failed sanitizer validation")]
    InvalidSanitizedContent,
}

#[derive(Clone, Eq, PartialEq)]
pub struct EncodedEntryContent {
    storage: String,
}

impl EncodedEntryContent {
    pub fn from_sanitized(content: &SanitizedContent) -> Result<Self, EntryContentError> {
        validate_stored_content(content.html(), content.images()).map_err(map_sanitize_error)?;
        let envelope = BorrowedEnvelope {
            html: content.html(),
            inert_images: content.images().iter().map(BorrowedImage::from).collect(),
        };
        validate_metadata_budget(&envelope.inert_images)?;
        let json = serde_json::to_string(&envelope).map_err(|_| EntryContentError::Malformed)?;
        let length = STORAGE_PREFIX
            .len()
            .checked_add(json.len())
            .ok_or(EntryContentError::EnvelopeTooLarge)?;
        if length > MAX_ENVELOPE_BYTES {
            return Err(EntryContentError::EnvelopeTooLarge);
        }
        let mut storage = String::with_capacity(length);
        storage.push_str(STORAGE_PREFIX);
        storage.push_str(&json);
        Ok(Self { storage })
    }

    #[must_use]
    pub fn as_storage_str(&self) -> &str {
        &self.storage
    }
}

impl fmt::Debug for EncodedEntryContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodedEntryContent")
            .field("bytes", &self.storage.len())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct EntryContentDetail {
    html: String,
    inert_images: Vec<InertImage>,
}

impl EntryContentDetail {
    pub fn decode(storage: &str) -> Result<Self, EntryContentError> {
        if storage.len() > MAX_ENVELOPE_BYTES {
            return Err(EntryContentError::EnvelopeTooLarge);
        }
        let Some(json) = storage.strip_prefix(STORAGE_PREFIX) else {
            return Err(EntryContentError::UnsupportedVersion);
        };
        let envelope: OwnedEnvelope =
            serde_json::from_str(json).map_err(|_| EntryContentError::Malformed)?;
        if envelope.html.len() > MAX_HTML_BYTES {
            return Err(EntryContentError::HtmlTooLarge);
        }
        if envelope.inert_images.len() > MAX_IMAGES {
            return Err(EntryContentError::TooManyImages);
        }
        validate_metadata_budget(&envelope.inert_images)?;
        let canonical =
            serde_json::to_string(&envelope).map_err(|_| EntryContentError::Malformed)?;
        if canonical != json {
            return Err(EntryContentError::NonCanonical);
        }

        let images = envelope
            .inert_images
            .into_iter()
            .map(|image| {
                InertImage::from_stored_parts(
                    image.image_index,
                    image.source_url,
                    image.alt,
                    image.width,
                    image.height,
                )
            })
            .collect::<Vec<_>>();
        validate_stored_content(&envelope.html, &images).map_err(map_sanitize_error)?;
        Ok(Self {
            html: envelope.html,
            inert_images: images,
        })
    }

    #[must_use]
    pub fn html(&self) -> &str {
        &self.html
    }

    #[must_use]
    pub fn inert_images(&self) -> &[InertImage] {
        &self.inert_images
    }
}

impl fmt::Debug for EntryContentDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntryContentDetail")
            .field("html_bytes", &self.html.len())
            .field("image_count", &self.inert_images.len())
            .finish()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BorrowedEnvelope<'a> {
    html: &'a str,
    inert_images: Vec<BorrowedImage<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BorrowedImage<'a> {
    image_index: u32,
    source_url: &'a str,
    alt: Option<&'a str>,
    width: Option<u32>,
    height: Option<u32>,
}

impl<'a> From<&'a InertImage> for BorrowedImage<'a> {
    fn from(image: &'a InertImage) -> Self {
        Self {
            image_index: image.image_index(),
            source_url: image.source_url(),
            alt: image.alt(),
            width: image.width(),
            height: image.height(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OwnedEnvelope {
    html: String,
    inert_images: Vec<OwnedImage>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OwnedImage {
    image_index: u32,
    source_url: String,
    alt: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

fn validate_metadata_budget<T: Serialize>(images: &[T]) -> Result<(), EntryContentError> {
    let bytes = serde_json::to_vec(images)
        .map_err(|_| EntryContentError::Malformed)?
        .len();
    if bytes > MAX_IMAGE_METADATA_BYTES {
        Err(EntryContentError::ImageMetadataTooLarge)
    } else {
        Ok(())
    }
}

fn map_sanitize_error(error: StoredContentValidationError) -> EntryContentError {
    match error {
        StoredContentValidationError::HtmlTooLong => EntryContentError::HtmlTooLarge,
        StoredContentValidationError::TooManyImages => EntryContentError::TooManyImages,
        StoredContentValidationError::ImageMetadataTooLarge => {
            EntryContentError::ImageMetadataTooLarge
        }
        StoredContentValidationError::ImageAltTooLong
        | StoredContentValidationError::ImageDimensionInvalid
        | StoredContentValidationError::ImageIndexInvalid
        | StoredContentValidationError::ImageMetadataMismatch
        | StoredContentValidationError::ImageSourceInvalid
        | StoredContentValidationError::NotIdempotent => EntryContentError::InvalidSanitizedContent,
    }
}
