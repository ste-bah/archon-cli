//! TASK-P0-B.1a (#178) Multi-modal content helpers.
//!
//! Validates image bytes against format-specific magic numbers and
//! produces Anthropic-compatible [`ContentBlock::Image`] blocks. Future
//! tickets (#179 PDF, #180 audio) extend this module with Document/Audio
//! variants.
//!
//! # Image formats
//!
//! Supported media types: `image/png`, `image/jpeg`, `image/gif`,
//! `image/webp`. Each is validated by its magic-byte signature. Invalid
//! bytes or mismatched media_type -> [`MultimodalError`].
//!
//! # Anthropic shape
//!
//! ```json
//! {
//!   "type": "image",
//!   "source": {
//!     "type": "base64",
//!     "media_type": "image/png",
//!     "data": "<base64>"
//!   }
//! }
//! ```

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::types::ContentBlock;

/// Source of an image content block (Anthropic schema).
///
/// `source_type` always serializes as the JSON field `"type"` and for the
/// current Anthropic API is always `"base64"`. Kept as a `String` so that
/// future source shapes (URL, file-id) can reuse the struct without a
/// breaking change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

/// Errors from multimodal content conversion.
#[derive(Debug, thiserror::Error)]
pub enum MultimodalError {
    #[error("empty input bytes")]
    EmptyInput,
    #[error(
        "unsupported media_type '{0}' (expected image/png, image/jpeg, image/gif, or image/webp)"
    )]
    UnsupportedMediaType(String),
    #[error("bytes do not match media_type '{0}' magic signature")]
    MagicMismatch(String),
}

/// PNG magic bytes: `[0x89, 'P', 'N', 'G', 0x0D, 0x0A, 0x1A, 0x0A]`.
pub(crate) const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
/// JPEG magic: `FF D8 FF`.
pub(crate) const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF];
/// GIF magic: "GIF87a".
pub(crate) const GIF87_MAGIC: &[u8] = b"GIF87a";
/// GIF magic: "GIF89a".
pub(crate) const GIF89_MAGIC: &[u8] = b"GIF89a";
/// WEBP RIFF container tag (bytes 0..4).
pub(crate) const WEBP_RIFF: &[u8] = b"RIFF";
/// WEBP form tag (bytes 8..12).
pub(crate) const WEBP_TAG: &[u8] = b"WEBP";

/// Build a [`ContentBlock::Image`] from raw bytes + a declared
/// `media_type`.
///
/// Validates that `bytes` starts with the magic signature of the declared
/// format; rejects unsupported or mismatched types. On success, the bytes
/// are base64-encoded into an [`ImageSource`] wrapped in
/// [`ContentBlock::Image`].
///
/// # Errors
///
/// - [`MultimodalError::EmptyInput`] if `bytes` is empty.
/// - [`MultimodalError::UnsupportedMediaType`] if `media_type` is not one
///   of the Anthropic-supported image types.
/// - [`MultimodalError::MagicMismatch`] if `bytes` does not start with the
///   expected magic for `media_type`.
pub fn image_block_from_bytes(
    bytes: &[u8],
    media_type: &str,
) -> Result<ContentBlock, MultimodalError> {
    if bytes.is_empty() {
        return Err(MultimodalError::EmptyInput);
    }
    let ok = match media_type {
        "image/png" => bytes.starts_with(PNG_MAGIC),
        "image/jpeg" => bytes.starts_with(JPEG_MAGIC),
        "image/gif" => bytes.starts_with(GIF87_MAGIC) || bytes.starts_with(GIF89_MAGIC),
        "image/webp" => {
            bytes.len() >= 12 && &bytes[0..4] == WEBP_RIFF && &bytes[8..12] == WEBP_TAG
        }
        other => return Err(MultimodalError::UnsupportedMediaType(other.to_string())),
    };
    if !ok {
        return Err(MultimodalError::MagicMismatch(media_type.to_string()));
    }
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(ContentBlock::Image {
        source: ImageSource {
            source_type: "base64".to_string(),
            media_type: media_type.to_string(),
            data,
        },
    })
}

/// Deterministic minimal 1x1 black PNG for tests (67 bytes).
///
/// Used by #178 Gate-5 smoke + by follow-up tickets as a reference
/// fixture. Keeping the bytes hand-crafted (no `image` crate) keeps the
/// dependency surface minimal.
#[cfg(test)]
pub(crate) const MINIMAL_PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR len + tag
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // width=1 height=1
    0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, // 8-bit RGB + CRC
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, // CRC + IDAT len + tag
    0x54, 0x08, 0x99, 0x63, 0xF8, 0xCF, 0xC0, 0xC0, // IDAT data
    0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x5B, 0x9C, // ... + CRC
    0x9A, 0x41, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, // CRC + IEND len + tag
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82, // IEND + CRC
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_block_from_minimal_png_ok() {
        let block = image_block_from_bytes(MINIMAL_PNG_1X1, "image/png")
            .expect("1x1 PNG must be accepted");
        match block {
            ContentBlock::Image { source } => {
                assert_eq!(source.media_type, "image/png");
                assert_eq!(source.source_type, "base64");
                assert!(!source.data.is_empty());
            }
            other => panic!("expected Image, got {:?}", other),
        }
    }

    #[test]
    fn image_block_empty_bytes_errors() {
        let err = image_block_from_bytes(&[], "image/png").unwrap_err();
        assert!(matches!(err, MultimodalError::EmptyInput));
    }

    #[test]
    fn image_block_unsupported_media_type_errors() {
        let err = image_block_from_bytes(MINIMAL_PNG_1X1, "image/bmp").unwrap_err();
        assert!(matches!(err, MultimodalError::UnsupportedMediaType(_)));
    }

    #[test]
    fn image_block_magic_mismatch_errors() {
        // PNG bytes but declared as JPEG -> magic signature check fails.
        let err = image_block_from_bytes(MINIMAL_PNG_1X1, "image/jpeg").unwrap_err();
        assert!(matches!(err, MultimodalError::MagicMismatch(_)));
    }

    #[test]
    fn image_block_roundtrip_serde() {
        let block = image_block_from_bytes(MINIMAL_PNG_1X1, "image/png").unwrap();
        let json = serde_json::to_value(&block).unwrap();
        assert_eq!(json["type"], "image");
        assert_eq!(json["source"]["type"], "base64");
        assert_eq!(json["source"]["media_type"], "image/png");
        let data = json["source"]["data"].as_str().unwrap();
        // Base64 decode -> must match original bytes exactly.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap();
        assert_eq!(decoded.as_slice(), MINIMAL_PNG_1X1);
    }

    #[test]
    fn image_block_jpeg_magic_accepted() {
        // Minimal JPEG APP0 header: FF D8 FF E0 00 10 ...
        let jpeg_stub = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        let block = image_block_from_bytes(&jpeg_stub, "image/jpeg").unwrap();
        assert!(matches!(block, ContentBlock::Image { .. }));
    }

    #[test]
    fn image_block_gif_both_variants_accepted() {
        let gif87 = b"GIF87a\x01\x00";
        let gif89 = b"GIF89a\x01\x00";
        assert!(image_block_from_bytes(gif87, "image/gif").is_ok());
        assert!(image_block_from_bytes(gif89, "image/gif").is_ok());
    }

    #[test]
    fn image_block_webp_accepted() {
        // RIFF <4-byte size> WEBP <fourcc> ...
        let webp = b"RIFF\x00\x00\x00\x08WEBPVP8 ";
        let block = image_block_from_bytes(webp, "image/webp").unwrap();
        assert!(matches!(block, ContentBlock::Image { .. }));
    }
}
