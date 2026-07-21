mod hash;
mod images;
mod policy;

use std::{error::Error, fmt};

pub(crate) use hash::{content_hash, source_content_hash};
use images::{
    extract_images, extract_search_text, extract_text, extract_translation_segments,
    validate_image_metadata,
};
use policy::sanitize_html;

const MAX_FINAL_HTML_BYTES: usize = 1024 * 1024;
const MAX_IMAGES: usize = 256;
const MAX_IMAGE_METADATA_BYTES: usize = 256 * 1024;
const MAX_SOURCE_URL_BYTES: usize = 4 * 1024;

#[derive(Clone, Eq, PartialEq)]
pub struct InertImage {
    image_index: u32,
    source_url: String,
    alt: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

impl InertImage {
    pub(crate) fn from_stored_parts(
        image_index: u32,
        source_url: String,
        alt: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
    ) -> Self {
        Self {
            image_index,
            source_url,
            alt,
            width,
            height,
        }
    }

    #[must_use]
    pub const fn image_index(&self) -> u32 {
        self.image_index
    }

    #[must_use]
    pub fn source_url(&self) -> &str {
        &self.source_url
    }

    #[must_use]
    pub fn alt(&self) -> Option<&str> {
        self.alt.as_deref()
    }

    #[must_use]
    pub const fn width(&self) -> Option<u32> {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> Option<u32> {
        self.height
    }
}

impl fmt::Debug for InertImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InertImage")
            .field("image_index", &self.image_index)
            .field("source_url", &"[REDACTED]")
            .field("alt", &self.alt.as_ref().map(|_| "[REDACTED]"))
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SanitizedContent {
    html: String,
    images: Vec<InertImage>,
}

impl SanitizedContent {
    #[must_use]
    pub fn html(&self) -> &str {
        &self.html
    }

    #[must_use]
    pub fn images(&self) -> &[InertImage] {
        &self.images
    }
}

impl fmt::Debug for SanitizedContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SanitizedContent")
            .field("html_bytes", &self.html.len())
            .field("image_count", &self.images.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SanitizeError {
    FinalHtmlTooLong { bytes: usize },
    TooManyImages { count: usize },
    ImageAltTooLong { bytes: usize },
    ImageDimensionInvalid,
    ImageMetadataTooLarge { bytes: usize },
}

impl fmt::Display for SanitizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FinalHtmlTooLong { bytes } => {
                write!(formatter, "sanitized HTML has {bytes} bytes")
            }
            Self::TooManyImages { count } => write!(formatter, "too many inert images: {count}"),
            Self::ImageAltTooLong { bytes } => write!(formatter, "image alt has {bytes} bytes"),
            Self::ImageDimensionInvalid => formatter.write_str("image dimension is invalid"),
            Self::ImageMetadataTooLarge { bytes } => {
                write!(formatter, "inert image metadata has {bytes} bytes")
            }
        }
    }
}

impl Error for SanitizeError {}

pub(crate) fn sanitize_entry_html(
    base_url: &str,
    input: &str,
) -> Result<SanitizedContent, SanitizeError> {
    let with_sources = sanitize_html(base_url, input, true);
    validate_image_metadata(&with_sources)?;
    let source_images = extract_images(&with_sources);
    let html = sanitize_html(base_url, &with_sources, false);
    if html.len() > MAX_FINAL_HTML_BYTES {
        return Err(SanitizeError::FinalHtmlTooLong { bytes: html.len() });
    }
    let final_images = extract_images(&html);
    if final_images.len() > MAX_IMAGES {
        return Err(SanitizeError::TooManyImages {
            count: final_images.len(),
        });
    }
    let images: Vec<_> = source_images
        .into_iter()
        .zip(final_images)
        .filter_map(|(source, final_image)| {
            source.source_url.map(|source_url| InertImage {
                image_index: final_image.image_index,
                source_url,
                alt: final_image.alt,
                width: final_image.width,
                height: final_image.height,
            })
        })
        .collect();
    let metadata_bytes = canonical_metadata_bytes(&images);
    if metadata_bytes > MAX_IMAGE_METADATA_BYTES {
        return Err(SanitizeError::ImageMetadataTooLarge {
            bytes: metadata_bytes,
        });
    }

    Ok(SanitizedContent { html, images })
}

pub(crate) fn resanitize_entry_html(
    base_url: &str,
    input: &str,
    inherited_images: &[InertImage],
) -> Result<SanitizedContent, SanitizeError> {
    let with_metadata = sanitize_html(base_url, input, true);
    validate_image_metadata(&with_metadata)?;
    let html = sanitize_html(base_url, &with_metadata, false);
    if html.len() > MAX_FINAL_HTML_BYTES {
        return Err(SanitizeError::FinalHtmlTooLong { bytes: html.len() });
    }
    let final_images = extract_images(&html);
    if final_images.len() > MAX_IMAGES {
        return Err(SanitizeError::TooManyImages {
            count: final_images.len(),
        });
    }
    let images: Vec<_> = inherited_images
        .iter()
        .filter_map(|image| {
            let final_image = final_images.get(image.image_index as usize)?;
            Some(InertImage {
                image_index: final_image.image_index,
                source_url: image.source_url.clone(),
                alt: final_image.alt.clone(),
                width: final_image.width,
                height: final_image.height,
            })
        })
        .collect();
    let metadata_bytes = canonical_metadata_bytes(&images);
    if metadata_bytes > MAX_IMAGE_METADATA_BYTES {
        return Err(SanitizeError::ImageMetadataTooLarge {
            bytes: metadata_bytes,
        });
    }
    Ok(SanitizedContent { html, images })
}

pub(crate) fn canonical_summary_text(input: &str) -> Option<String> {
    let decoded = extract_text(input);
    let sanitized = sanitize_html("https://stored-summary.invalid/", &decoded, false);
    let text = extract_text(&sanitized);
    let mut normalized = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
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
    (!normalized.is_empty()).then_some(normalized)
}

pub(crate) fn extract_rendered_text(input: &str) -> String {
    extract_search_text(input)
}

pub(crate) fn rendered_translation_segments(input: &str) -> Vec<String> {
    extract_translation_segments(input)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoredContentValidationError {
    HtmlTooLong,
    TooManyImages,
    ImageAltTooLong,
    ImageDimensionInvalid,
    ImageMetadataTooLarge,
    ImageIndexInvalid,
    ImageMetadataMismatch,
    ImageSourceInvalid,
    NotIdempotent,
}

pub(crate) fn validate_stored_content(
    html: &str,
    images: &[InertImage],
) -> Result<(), StoredContentValidationError> {
    if html.len() > MAX_FINAL_HTML_BYTES {
        return Err(StoredContentValidationError::HtmlTooLong);
    }
    validate_image_metadata(html).map_err(|error| match error {
        SanitizeError::ImageAltTooLong { .. } => StoredContentValidationError::ImageAltTooLong,
        SanitizeError::ImageDimensionInvalid => StoredContentValidationError::ImageDimensionInvalid,
        _ => StoredContentValidationError::ImageMetadataMismatch,
    })?;
    if sanitize_html("https://stored-content.invalid/", html, false) != html {
        return Err(StoredContentValidationError::NotIdempotent);
    }

    let final_images = extract_images(html);
    if final_images.len() > MAX_IMAGES {
        return Err(StoredContentValidationError::TooManyImages);
    }

    let mut previous_index = None;
    for image in images {
        let index = usize::try_from(image.image_index)
            .map_err(|_| StoredContentValidationError::ImageIndexInvalid)?;
        if previous_index.is_some_and(|previous| image.image_index <= previous) {
            return Err(StoredContentValidationError::ImageIndexInvalid);
        }
        previous_index = Some(image.image_index);

        let final_image = final_images
            .get(index)
            .ok_or(StoredContentValidationError::ImageIndexInvalid)?;
        if final_image.alt != image.alt
            || final_image.width != image.width
            || final_image.height != image.height
        {
            return Err(StoredContentValidationError::ImageMetadataMismatch);
        }
        validate_source_url(&image.source_url)?;
    }
    if canonical_metadata_bytes(images) > MAX_IMAGE_METADATA_BYTES {
        return Err(StoredContentValidationError::ImageMetadataTooLarge);
    }
    Ok(())
}

fn validate_source_url(source_url: &str) -> Result<(), StoredContentValidationError> {
    if source_url.len() > MAX_SOURCE_URL_BYTES {
        return Err(StoredContentValidationError::ImageSourceInvalid);
    }
    let url = url::Url::parse(source_url)
        .map_err(|_| StoredContentValidationError::ImageSourceInvalid)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.as_str() != source_url
    {
        return Err(StoredContentValidationError::ImageSourceInvalid);
    }
    Ok(())
}

fn canonical_metadata_bytes(images: &[InertImage]) -> usize {
    images.iter().fold(0_usize, |total, image| {
        total
            .saturating_add(4)
            .saturating_add(4 + image.source_url.len())
            .saturating_add(1 + image.alt.as_ref().map_or(0, |alt| 4 + alt.len()))
            .saturating_add(1 + usize::from(image.width.is_some()) * 4)
            .saturating_add(1 + usize::from(image.height.is_some()) * 4)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_segments_preserve_repeated_adjacent_paragraphs() {
        assert_eq!(
            rendered_translation_segments("<p>Repeat me</p><p>Repeat me</p>"),
            ["Repeat me", "Repeat me"]
        );
    }

    #[test]
    fn sanitizer_enforces_image_and_final_html_budgets() {
        let base = "https://example.test/";
        let accepted = format!(
            "<img src='/x' alt='{}' width='16384' height='1'>",
            "a".repeat(4096)
        );
        assert!(sanitize_entry_html(base, &accepted).is_ok());

        let alt = format!("<img src='/x' alt='{}'>", "a".repeat(4097));
        assert_eq!(
            sanitize_entry_html(base, &alt).expect_err("alt cap"),
            SanitizeError::ImageAltTooLong { bytes: 4097 }
        );
        for dimension in ["0", "16385", "not-a-number"] {
            let html = format!("<img src='/x' width='{dimension}'>");
            assert_eq!(
                sanitize_entry_html(base, &html).expect_err("dimension cap"),
                SanitizeError::ImageDimensionInvalid
            );
        }
        let images = (0..257)
            .map(|index| format!("<img src='/images/{index}.jpg' alt='{index}'>"))
            .collect::<String>();
        assert_eq!(
            sanitize_entry_html(base, &images).expect_err("image count"),
            SanitizeError::TooManyImages { count: 257 }
        );
        let metadata = (0..65)
            .map(|index| {
                format!(
                    "<img src='https://img.example.test/{}/{}' alt='{index}'>",
                    "x".repeat(4_000),
                    index
                )
            })
            .collect::<String>();
        assert!(matches!(
            sanitize_entry_html(base, &metadata),
            Err(SanitizeError::ImageMetadataTooLarge { .. })
        ));
        let oversized = format!("<p>{}</p>", "x".repeat(1024 * 1024));
        assert!(matches!(
            sanitize_entry_html(base, &oversized),
            Err(SanitizeError::FinalHtmlTooLong { .. })
        ));
    }
}
