use crate::errors::DocsError;

pub fn detect_mime(image_bytes: &[u8]) -> Result<&'static str, DocsError> {
    if image_bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Ok("image/png");
    }
    if image_bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok("image/jpeg");
    }
    if image_bytes.starts_with(b"GIF") {
        return Ok("image/gif");
    }
    if image_bytes.len() >= 12 && &image_bytes[0..4] == b"RIFF" && &image_bytes[8..12] == b"WEBP" {
        return Ok("image/webp");
    }
    Err(DocsError::VlmProvider {
        provider: "mime".into(),
        message: "unknown image signature; refusing to send image with guessed MIME type".into(),
        status_code: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_mime_recognises_png_jpeg_webp_gif() {
        assert_eq!(detect_mime(&[0x89, b'P', b'N', b'G']).unwrap(), "image/png");
        assert_eq!(detect_mime(&[0xFF, 0xD8, 0xFF]).unwrap(), "image/jpeg");
        assert_eq!(detect_mime(b"GIF89a").unwrap(), "image/gif");
        assert_eq!(detect_mime(b"RIFFxxxxWEBPpayload").unwrap(), "image/webp");
    }

    #[test]
    fn detect_mime_fails_on_unknown_signature() {
        let err = detect_mime(b"not-an-image").unwrap_err();
        assert!(matches!(err, DocsError::VlmProvider { provider, .. } if provider == "mime"));
    }
}
