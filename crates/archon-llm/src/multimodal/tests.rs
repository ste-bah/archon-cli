//! Tests for the multimodal module (image, document, audio).
//!
//! Split out from `multimodal.rs` to keep that file under 500 lines after
//! the TASK-P0-B.1c (#180) audio surface was added. Uses `super::*` to
//! pull in the parent module's `pub(crate)` fixtures and `pub` API.

use super::*;
use base64::Engine;

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

// -----------------------------------------------------------------
// TASK-P0-B.1b (#179) PDF document tests
// -----------------------------------------------------------------

#[test]
fn document_block_from_minimal_pdf_ok() {
    let block = document_block_from_bytes(MINIMAL_PDF_EMPTY_PAGE, "application/pdf")
        .expect("minimal PDF must be accepted");
    match block {
        ContentBlock::Document { source } => {
            assert_eq!(source.media_type, "application/pdf");
            assert_eq!(source.source_type, "base64");
            assert!(!source.data.is_empty());
        }
        other => panic!("expected Document, got {:?}", other),
    }
}

#[test]
fn document_block_empty_bytes_errors() {
    let err = document_block_from_bytes(&[], "application/pdf").unwrap_err();
    assert!(matches!(err, MultimodalError::EmptyInput));
}

#[test]
fn document_block_unsupported_media_type_errors() {
    // Even a well-formed PDF declared as `text/plain` must be rejected.
    let err = document_block_from_bytes(MINIMAL_PDF_EMPTY_PAGE, "text/plain").unwrap_err();
    assert!(matches!(err, MultimodalError::UnsupportedMediaType(_)));
}

#[test]
fn document_block_magic_mismatch_errors() {
    // Non-PDF bytes declared as PDF -> magic signature check fails.
    let not_pdf = b"not a pdf, just ASCII";
    let err = document_block_from_bytes(not_pdf, "application/pdf").unwrap_err();
    assert!(matches!(err, MultimodalError::MagicMismatch(_)));
}

#[test]
fn document_block_roundtrip_serde() {
    let block =
        document_block_from_bytes(MINIMAL_PDF_EMPTY_PAGE, "application/pdf").unwrap();
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "document");
    assert_eq!(json["source"]["type"], "base64");
    assert_eq!(json["source"]["media_type"], "application/pdf");
    let data = json["source"]["data"].as_str().unwrap();
    // Base64 decode -> must match original bytes exactly.
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    assert_eq!(decoded.as_slice(), MINIMAL_PDF_EMPTY_PAGE);
}

#[test]
fn document_block_pdf_magic_constant_matches_header() {
    // Sanity: the PDF_MAGIC constant must agree with the fixture header.
    assert!(MINIMAL_PDF_EMPTY_PAGE.starts_with(PDF_MAGIC));
}

// -----------------------------------------------------------------
// TASK-P0-B.1c (#180) audio tests
// -----------------------------------------------------------------

#[test]
fn audio_block_from_minimal_wav_ok() {
    let block = audio_block_from_bytes(MINIMAL_WAV_SILENT, "audio/wav")
        .expect("minimal WAV must be accepted");
    match block {
        ContentBlock::Audio { source } => {
            assert_eq!(source.media_type, "audio/wav");
            assert_eq!(source.source_type, "base64");
            assert!(!source.data.is_empty());
        }
        other => panic!("expected Audio, got {:?}", other),
    }
}

#[test]
fn audio_block_empty_bytes_errors() {
    let err = audio_block_from_bytes(&[], "audio/wav").unwrap_err();
    assert!(matches!(err, MultimodalError::EmptyInput));
}

#[test]
fn audio_block_unsupported_media_type_errors() {
    let err = audio_block_from_bytes(MINIMAL_WAV_SILENT, "audio/m4a").unwrap_err();
    assert!(matches!(err, MultimodalError::UnsupportedMediaType(_)));
}

#[test]
fn audio_block_magic_mismatch_errors() {
    // Non-WAV bytes (but long enough to pass length check) declared as WAV.
    let not_wav = b"NOTRIFF\x00\x00\x00\x00\x00";
    let err = audio_block_from_bytes(not_wav, "audio/wav").unwrap_err();
    assert!(matches!(err, MultimodalError::MagicMismatch(_)));
}

#[test]
fn audio_block_roundtrip_serde() {
    let block = audio_block_from_bytes(MINIMAL_WAV_SILENT, "audio/wav").unwrap();
    let json = serde_json::to_value(&block).unwrap();
    assert_eq!(json["type"], "audio");
    assert_eq!(json["source"]["type"], "base64");
    assert_eq!(json["source"]["media_type"], "audio/wav");
    let data = json["source"]["data"].as_str().unwrap();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    assert_eq!(decoded.as_slice(), MINIMAL_WAV_SILENT);
}

#[test]
fn audio_block_wav_44_bytes_exactly() {
    // RIFF/WAVE minimal valid file is exactly 44 bytes.
    assert_eq!(MINIMAL_WAV_SILENT.len(), 44);
}

#[test]
fn audio_block_mp3_id3_accepted() {
    let mp3 = b"ID3\x03\x00";
    let block = audio_block_from_bytes(mp3, "audio/mp3").unwrap();
    assert!(matches!(block, ContentBlock::Audio { .. }));
}

#[test]
fn audio_block_mp3_frame_sync_accepted() {
    let mp3 = [0xFFu8, 0xFB, 0x90, 0x00];
    let block = audio_block_from_bytes(&mp3, "audio/mpeg").unwrap();
    assert!(matches!(block, ContentBlock::Audio { .. }));
}

#[test]
fn audio_block_ogg_accepted() {
    let ogg = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00";
    let block = audio_block_from_bytes(ogg, "audio/ogg").unwrap();
    assert!(matches!(block, ContentBlock::Audio { .. }));
}

#[test]
fn audio_block_flac_accepted() {
    let flac = b"fLaC\x00\x00\x00\x22";
    let block = audio_block_from_bytes(flac, "audio/flac").unwrap();
    assert!(matches!(block, ContentBlock::Audio { .. }));
}
