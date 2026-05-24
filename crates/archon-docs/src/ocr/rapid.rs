use std::path::Path;
use std::time::Instant;

use serde::Deserialize;

use super::provider::OcrExtractResult;
use crate::errors::DocsError;
use crate::models::PageOffset;

pub async fn extract_image_with_rapidocr(path: &Path) -> Result<OcrExtractResult, DocsError> {
    let started = Instant::now();
    let output = tokio::process::Command::new(python_bin())
        .arg("-c")
        .arg(RAPID_OCR_SCRIPT)
        .arg(path)
        .arg(min_score().to_string())
        .output()
        .await
        .map_err(|e| DocsError::OcrApi {
            message: format!("start RapidOCR Python fallback: {e}"),
            status_code: None,
        })?;

    if !output.status.success() {
        return Err(DocsError::OcrApi {
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            status_code: Some(output.status.code().unwrap_or(1) as u16),
        });
    }

    let parsed: RapidOutput =
        serde_json::from_slice(&output.stdout).map_err(|e| DocsError::OcrApi {
            message: format!("parse RapidOCR output: {e}"),
            status_code: None,
        })?;
    if parsed.text.trim().is_empty() {
        return Err(DocsError::OcrApi {
            message: "RapidOCR produced no text".into(),
            status_code: None,
        });
    }
    Ok(OcrExtractResult {
        page_count: 1,
        page_offsets: vec![PageOffset {
            page: 1,
            char_start: 0,
            char_end: parsed.text.len(),
        }],
        full_text: parsed.text,
        processing_duration_ms: started.elapsed().as_millis() as u64,
    })
}

pub fn prefer_rapidocr() -> bool {
    std::env::var("ARCHON_OCR_ENGINE")
        .map(|value| value.trim().eq_ignore_ascii_case("rapidocr"))
        .unwrap_or(false)
}

pub fn allow_rapidocr_fallback() -> bool {
    std::env::var("ARCHON_OCR_ENGINE")
        .map(|value| !value.trim().eq_ignore_ascii_case("tesseract"))
        .unwrap_or(true)
}

fn python_bin() -> String {
    std::env::var("ARCHON_RAPIDOCR_PYTHON")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "python3".into())
}

fn min_score() -> f32 {
    std::env::var("ARCHON_RAPIDOCR_MIN_SCORE")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(0.55)
}

#[derive(Debug, Deserialize)]
struct RapidOutput {
    text: String,
}

const RAPID_OCR_SCRIPT: &str = r#"
import json
import re
import sys

try:
    from rapidocr_onnxruntime import RapidOCR
except Exception:
    try:
        from rapidocr import RapidOCR
    except Exception as exc:
        print(f"RapidOCR is not available: {exc}", file=sys.stderr)
        sys.exit(2)

image_path = sys.argv[1]
min_score = float(sys.argv[2])
engine = RapidOCR()
raw = engine(image_path)
rows = raw[0] if isinstance(raw, tuple) else raw

seen = {}
for row in rows or []:
    text = ""
    score = 1.0
    if isinstance(row, (list, tuple)):
        if len(row) >= 3:
            text, score = row[1], row[2]
        elif len(row) >= 2:
            text, score = row[0], row[1]
    text = re.sub(r"\s+", " ", str(text)).strip()
    if not text or len(text) < 2:
        continue
    if re.fullmatch(r"[\d:.,/\- ]{2,}", text):
        continue
    try:
        score = float(score)
    except Exception:
        score = 1.0
    if score < min_score:
        continue
    key = text.lower()
    if key not in seen or score > seen[key][1]:
        seen[key] = (text, score)

lines = [item[0] for item in sorted(seen.values(), key=lambda item: item[1], reverse=True)]
print(json.dumps({"text": "\n".join(lines)}, ensure_ascii=False))
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapidocr_env_defaults_to_tesseract_first_with_fallback() {
        unsafe {
            std::env::remove_var("ARCHON_OCR_ENGINE");
        }
        assert!(!prefer_rapidocr());
        assert!(allow_rapidocr_fallback());
    }
}
