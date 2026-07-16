mod hash;
mod images;
mod policy;

use std::{error::Error, fmt};

pub(crate) use hash::{content_hash, source_content_hash};
use images::{extract_images, validate_image_metadata};
use policy::sanitize_html;

const MAX_FINAL_HTML_BYTES: usize = 1024 * 1024;
const MAX_IMAGES: usize = 256;
const MAX_IMAGE_METADATA_BYTES: usize = 256 * 1024;

#[derive(Clone, Eq, PartialEq)]
pub struct InertImage {
    image_index: u32,
    source_url: String,
    alt: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

impl InertImage {
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
