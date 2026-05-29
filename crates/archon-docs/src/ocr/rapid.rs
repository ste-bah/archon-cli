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

    let parsed = parse_rapid_stdout(&output.stdout)?;
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

fn parse_rapid_stdout(stdout: &[u8]) -> Result<RapidOutput, DocsError> {
    serde_json::from_slice(stdout).or_else(|full_error| {
        let stdout = String::from_utf8_lossy(stdout);
        let Some(line) = stdout
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| line.starts_with('{') && line.ends_with('}'))
        else {
            return Err(DocsError::OcrApi {
                message: format!("parse RapidOCR output: {full_error}"),
                status_code: None,
            });
        };
        serde_json::from_str(line).map_err(|line_error| DocsError::OcrApi {
            message: format!(
                "parse RapidOCR output: {full_error}; fallback JSON line failed: {line_error}"
            ),
            status_code: None,
        })
    })
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

def rows_from_output(raw):
    if isinstance(raw, tuple):
        raw = raw[0]
    if raw is None:
        return []

    txts = getattr(raw, "txts", None)
    if txts is not None:
        scores = getattr(raw, "scores", None) or []
        if len(scores) < len(txts):
            scores = list(scores) + [1.0] * (len(txts) - len(scores))
        return list(zip(txts, scores))

    to_json = getattr(raw, "to_json", None)
    if callable(to_json):
        json_rows = to_json() or []
        if isinstance(json_rows, str):
            try:
                json_rows = json.loads(json_rows)
            except Exception:
                json_rows = []
        rows = []
        for item in json_rows:
            if not isinstance(item, dict):
                continue
            text = (
                item.get("text")
                or item.get("txt")
                or item.get("transcription")
                or item.get("rec_text")
                or item.get("label")
                or ""
            )
            score = item.get("score") or item.get("confidence") or item.get("rec_score") or 1.0
            rows.append((text, score))
        if rows:
            return rows

    try:
        return list(raw)
    except TypeError:
        return []

def row_text_score(row):
    text = ""
    score = 1.0
    if isinstance(row, dict):
        text = (
            row.get("text")
            or row.get("txt")
            or row.get("transcription")
            or row.get("rec_text")
            or row.get("label")
            or ""
        )
        score = row.get("score") or row.get("confidence") or row.get("rec_score") or 1.0
    elif isinstance(row, (list, tuple)):
        if len(row) >= 3:
            text, score = row[1], row[2]
        elif len(row) >= 2 and isinstance(row[1], (list, tuple)) and len(row[1]) >= 2:
            text, score = row[1][0], row[1][1]
        elif len(row) >= 2:
            text, score = row[0], row[1]
        elif len(row) == 1:
            text = row[0]
    else:
        text = row
    return text, score

seen = {}
for row in rows_from_output(raw):
    text, score = row_text_score(row)
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
    use std::fs;

    #[test]
    fn rapidocr_env_defaults_to_tesseract_first_with_fallback() {
        unsafe {
            std::env::remove_var("ARCHON_OCR_ENGINE");
        }
        assert!(!prefer_rapidocr());
        assert!(allow_rapidocr_fallback());
    }

    #[test]
    fn rapidocr_parser_accepts_last_json_line_after_logs() {
        let parsed = parse_rapid_stdout(
            br#"[INFO] noisy rapidocr log line
{"text":"chart label"}
"#,
        )
        .unwrap();

        assert_eq!(parsed.text, "chart label");
    }

    #[tokio::test]
    async fn rapidocr_new_output_object_shape_is_supported() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("rapidocr");
        fs::create_dir(&package).unwrap();
        fs::write(
            package.join("__init__.py"),
            r#"
class RapidOCROutput:
    def __init__(self):
        self.txts = ("Keep this text", "12/34")
        self.scores = (0.92, 0.99)

class RapidOCR:
    def __call__(self, image_path):
        return RapidOCROutput()
"#,
        )
        .unwrap();
        let image = temp.path().join("image.png");
        fs::write(&image, b"not-really-an-image").unwrap();

        let _python_guard = EnvGuard::set("ARCHON_RAPIDOCR_PYTHON", "python3");
        let _pythonpath_guard = EnvGuard::set("PYTHONPATH", temp.path().to_string_lossy().as_ref());
        let _engine_guard = EnvGuard::remove("ARCHON_OCR_ENGINE");

        let result = extract_image_with_rapidocr(&image).await.unwrap();
        assert_eq!(result.full_text, "Keep this text");
    }

    struct EnvGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }

        fn remove(key: &'static str) -> Self {
            let old = std::env::var(key).ok();
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(value) = self.old.as_ref() {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }
}
