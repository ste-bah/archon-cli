use super::*;
use crate::embed::{self, LocalEmbeddingProvider};
use crate::models::PageOffset;

pub(super) fn test_db() -> DbInstance {
    let path = format!("/tmp/test-ingest-{}.db", uuid::Uuid::new_v4());
    let db = DbInstance::new("sqlite", &path, "").unwrap();
    ensure_doc_schema(&db).unwrap();
    db
}

#[cfg(unix)]
pub(super) fn png_bytes(width: u32, height: u32, payload_len: usize) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
    bytes.resize(payload_len.max(64), 0x42);
    bytes
}

#[cfg(unix)]
pub(super) fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, body).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

#[cfg(unix)]
pub(super) fn set_pdf_command_env(pdftotext: &Path, pdfimages: &Path, pdftoppm: &Path) {
    unsafe {
        std::env::set_var("ARCHON_PDFTOTEXT_BIN", pdftotext);
        std::env::set_var("ARCHON_PDFIMAGES_BIN", pdfimages);
        std::env::set_var("ARCHON_PDFTOPPM_BIN", pdftoppm);
    }
}

#[cfg(unix)]
pub(super) struct PdfCommandEnvGuard;

#[cfg(unix)]
impl Drop for PdfCommandEnvGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("ARCHON_PDFTOTEXT_BIN");
            std::env::remove_var("ARCHON_PDFIMAGES_BIN");
            std::env::remove_var("ARCHON_PDFTOPPM_BIN");
        }
    }
}

pub(super) fn vlm_enabled_policy() -> archon_policy::EffectivePolicy {
    let mut policy = archon_policy::EffectivePolicy::default();
    policy.docs.vlm.enabled = true;
    policy.docs.vlm.mode = "local".into();
    policy.docs.vlm.provider = "ollama".into();
    policy.workers.vlm = "allow-local".into();
    policy
}

pub(super) struct MockOcrProvider {
    pub(super) text: &'static str,
}

#[async_trait::async_trait]
impl OcrProvider for MockOcrProvider {
    async fn extract(
        &self,
        _request: OcrRequest,
    ) -> Result<crate::ocr::provider::OcrExtractResult, DocsError> {
        Ok(crate::ocr::provider::OcrExtractResult {
            full_text: self.text.to_string(),
            page_count: 1,
            page_offsets: vec![PageOffset {
                page: 1,
                char_start: 0,
                char_end: self.text.len(),
            }],
            processing_duration_ms: 7,
        })
    }

    fn name(&self) -> &'static str {
        "mock-ocr"
    }
}

pub(super) struct MockVlmProvider {
    pub(super) description: &'static str,
}

impl crate::vlm::VlmDescriptionProvider for MockVlmProvider {
    fn describe_image(
        &self,
        _image_bytes: &[u8],
        _prompt: Option<&str>,
    ) -> Result<String, DocsError> {
        Ok(self.description.to_string())
    }
}

pub(super) struct FailingVlmProvider;

impl crate::vlm::VlmDescriptionProvider for FailingVlmProvider {
    fn describe_image(
        &self,
        _image_bytes: &[u8],
        _prompt: Option<&str>,
    ) -> Result<String, DocsError> {
        Err(DocsError::OcrApi {
            message: "synthetic VLM outage".into(),
            status_code: None,
        })
    }
}

pub(super) struct FailsOnceVlmProvider {
    pub(super) calls: std::sync::atomic::AtomicUsize,
}

impl crate::vlm::VlmDescriptionProvider for FailsOnceVlmProvider {
    fn describe_image(
        &self,
        _image_bytes: &[u8],
        _prompt: Option<&str>,
    ) -> Result<String, DocsError> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            return Err(DocsError::OcrApi {
                message: "synthetic first-image VLM failure".into(),
                status_code: None,
            });
        }
        Ok("second chart description survives".into())
    }
}

pub(super) struct ProviderErrorThenOkVlmProvider {
    pub(super) calls: std::sync::atomic::AtomicUsize,
}

impl crate::vlm::VlmDescriptionProvider for ProviderErrorThenOkVlmProvider {
    fn describe_image(
        &self,
        _image_bytes: &[u8],
        _prompt: Option<&str>,
    ) -> Result<String, DocsError> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            return Err(DocsError::VlmProvider {
                provider: "openai-compat".into(),
                message: "chat completions response did not contain text".into(),
                status_code: None,
            });
        }
        Ok("chart description after retry".into())
    }
}

pub(super) fn reset_multimodal_test_providers() {
    crate::ocr::provider::clear_provider();
    crate::vlm::clear_provider();
    embed::clear_provider();
}

pub(super) struct IndexingMockProvider {
    pub(super) dim: usize,
    // If set, embed_chunks returns this error.
    pub(super) fail_with: Option<String>,
}

impl LocalEmbeddingProvider for IndexingMockProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        if let Some(ref msg) = self.fail_with {
            return Err(DocsError::Embedding {
                message: msg.clone(),
            });
        }
        Ok(chunks
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let mut v = vec![0.0_f32; self.dim];
                for (j, b) in c.bytes().enumerate() {
                    v[j % self.dim] = (b as f32) / 255.0;
                }
                v[0] = (i as f32 + 1.0) * 0.5;
                let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
                v.iter_mut().for_each(|x| *x /= norm);
                v
            })
            .collect())
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
        let mut results = self.embed_chunks(&[query.to_string()])?;
        Ok(results.remove(0))
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn backend_name(&self) -> &'static str {
        "mock-indexing"
    }
}

pub(super) struct MultimodalMockProvider {
    pub(super) dim: usize,
}

impl LocalEmbeddingProvider for MultimodalMockProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        Ok(chunks
            .iter()
            .map(|_| vec![0.5_f32, 0.5, 0.5, 0.5][..self.dim].to_vec())
            .collect())
    }

    fn embed_query(&self, _query: &str) -> Result<Vec<f32>, DocsError> {
        Ok(vec![0.5_f32, 0.5, 0.5, 0.5][..self.dim].to_vec())
    }

    fn embed_image(&self, _image_bytes: &[u8]) -> Result<Option<Vec<f32>>, DocsError> {
        Ok(Some(vec![0.25_f32, 0.25, 0.25, 0.25][..self.dim].to_vec()))
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn backend_name(&self) -> &'static str {
        "mock-multimodal"
    }
}
