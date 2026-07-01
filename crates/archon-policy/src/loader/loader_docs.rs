use serde::Deserialize;

use crate::models::*;

#[derive(Debug, Default, Deserialize)]
pub(super) struct RawDocsPolicy {
    vlm: Option<RawVlmPolicy>,
    pdf: Option<RawPdfPolicy>,
    retrieval: Option<RawRetrievalPolicy>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct RawVlmPolicy {
    enabled: Option<bool>,
    mode: Option<String>,
    provider: Option<String>,
    allow_cloud: Option<bool>,
    require_user_confirmation_for_cloud: Option<bool>,
    ollama: Option<RawOllamaVlmPolicy>,
    gemini: Option<RawGeminiVlmPolicy>,
    anthropic: Option<RawAnthropicVlmPolicy>,
    openai_compat: Option<RawOpenAiCompatVlmPolicy>,
}

#[derive(Debug, Default, Deserialize)]
struct RawOllamaVlmPolicy {
    endpoint: Option<String>,
    model: Option<String>,
    timeout_secs: Option<u64>,
    num_ctx: Option<u32>,
    keep_alive: Option<String>,
    num_gpu: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeminiVlmPolicy {
    api_key_env: Option<String>,
    model: Option<String>,
    endpoint_base: Option<String>,
    rpm_limit: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawAnthropicVlmPolicy {
    model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawOpenAiCompatVlmPolicy {
    endpoint: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
    timeout_secs: Option<u64>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawPdfPolicy {
    extract_embedded_images: Option<bool>,
    min_image_dimension: Option<u32>,
    min_image_bytes: Option<u64>,
    vlm_per_page_image: Option<bool>,
    render_text_pdf_pages: Option<bool>,
    image_enrichment_workers: Option<u32>,
    chunker: Option<String>,
    marker_sidecar: Option<String>,
    marker_device: Option<String>,
    marker_python: Option<String>,
    marker_memory_budget_mb: Option<u64>,
    scan_detector: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRetrievalPolicy {
    exact_weight: Option<f64>,
    semantic_weight: Option<f64>,
}

pub(super) fn apply_docs(policy: &mut DocsPolicy, raw: RawDocsPolicy) {
    if let Some(vlm) = raw.vlm {
        apply_legacy_vlm(&mut policy.vlm, vlm);
    }
    if let Some(pdf) = raw.pdf {
        apply_pdf(&mut policy.pdf, pdf);
    }
    if let Some(retrieval) = raw.retrieval {
        apply_retrieval(&mut policy.retrieval, retrieval);
    }
}

pub(super) fn apply_legacy_vlm(policy: &mut VlmPolicy, raw: RawVlmPolicy) {
    if let Some(value) = raw.enabled {
        policy.enabled = value;
    }
    if let Some(value) = raw.mode {
        policy.mode = value;
    }
    if let Some(value) = raw.provider {
        policy.provider = value;
    }
    if let Some(value) = raw.allow_cloud {
        policy.allow_cloud = value;
    }
    if let Some(value) = raw.require_user_confirmation_for_cloud {
        policy.require_user_confirmation_for_cloud = value;
    }
    if let Some(value) = raw.ollama {
        apply_ollama_vlm(&mut policy.ollama, value);
    }
    if let Some(value) = raw.gemini {
        apply_gemini_vlm(&mut policy.gemini, value);
    }
    if let Some(value) = raw.anthropic {
        apply_anthropic_vlm(&mut policy.anthropic, value);
    }
    if let Some(value) = raw.openai_compat {
        apply_openai_compat_vlm(&mut policy.openai_compat, value);
    }
}

fn apply_ollama_vlm(policy: &mut OllamaVlmPolicy, raw: RawOllamaVlmPolicy) {
    if let Some(value) = raw.endpoint {
        policy.endpoint = value;
    }
    if let Some(value) = raw.model {
        policy.model = value;
    }
    if let Some(value) = raw.timeout_secs {
        policy.timeout_secs = value;
    }
    if raw.num_ctx.is_some() {
        policy.num_ctx = raw.num_ctx;
    }
    if raw.keep_alive.is_some() {
        policy.keep_alive = raw.keep_alive;
    }
    if raw.num_gpu.is_some() {
        policy.num_gpu = raw.num_gpu;
    }
}

fn apply_gemini_vlm(policy: &mut GeminiVlmPolicy, raw: RawGeminiVlmPolicy) {
    if let Some(value) = raw.api_key_env {
        policy.api_key_env = value;
    }
    if let Some(value) = raw.model {
        policy.model = value;
    }
    if let Some(value) = raw.endpoint_base {
        policy.endpoint_base = value;
    }
    if let Some(value) = raw.rpm_limit {
        policy.rpm_limit = value;
    }
}

fn apply_anthropic_vlm(policy: &mut AnthropicVlmPolicy, raw: RawAnthropicVlmPolicy) {
    if let Some(value) = raw.model {
        policy.model = value;
    }
}

fn apply_openai_compat_vlm(policy: &mut OpenAiCompatVlmPolicy, raw: RawOpenAiCompatVlmPolicy) {
    if let Some(value) = raw.endpoint {
        policy.endpoint = value;
    }
    if let Some(value) = raw.model {
        policy.model = value;
    }
    if let Some(value) = raw.api_key_env {
        policy.api_key_env = value;
    }
    if let Some(value) = raw.timeout_secs {
        policy.timeout_secs = value;
    }
    if let Some(value) = raw.max_tokens {
        policy.max_tokens = value;
    }
    if let Some(value) = raw.temperature {
        policy.temperature = value;
    }
}

fn apply_pdf(policy: &mut PdfPolicy, raw: RawPdfPolicy) {
    if let Some(value) = raw.extract_embedded_images {
        policy.extract_embedded_images = value;
    }
    if let Some(value) = raw.min_image_dimension {
        policy.min_image_dimension = value;
    }
    if let Some(value) = raw.min_image_bytes {
        policy.min_image_bytes = value;
    }
    if let Some(value) = raw.vlm_per_page_image {
        policy.vlm_per_page_image = value;
    }
    if let Some(value) = raw.render_text_pdf_pages {
        policy.render_text_pdf_pages = value;
    }
    if let Some(value) = raw.image_enrichment_workers {
        policy.image_enrichment_workers = value.clamp(1, 16);
    }
    if let Some(value) = raw.chunker {
        // Only accept known strategies; anything else keeps the default (token_aware).
        if value == "page_anchor" || value == "token_aware" {
            policy.chunker = value;
        }
    }
    if raw.marker_sidecar.is_some() {
        policy.marker_sidecar = raw.marker_sidecar;
    }
    if raw.marker_device.is_some() {
        policy.marker_device = raw.marker_device;
    }
    if raw.marker_python.is_some() {
        policy.marker_python = raw.marker_python;
    }
    if raw.marker_memory_budget_mb.is_some() {
        policy.marker_memory_budget_mb = raw.marker_memory_budget_mb;
    }
    if let Some(value) = raw.scan_detector {
        // Only accept known detectors; anything else keeps the default (aspect).
        if value == "aspect" || value == "coverage" || value == "union" {
            policy.scan_detector = value;
        }
    }
}

fn apply_retrieval(policy: &mut RetrievalPolicy, raw: RawRetrievalPolicy) {
    if let Some(value) = raw.exact_weight {
        policy.exact_weight = value;
    }
    if let Some(value) = raw.semantic_weight {
        policy.semantic_weight = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_detector_coverage_is_applied() {
        let mut pdf = PdfPolicy::default();
        assert_eq!(pdf.scan_detector, "aspect", "default guard");
        apply_pdf(
            &mut pdf,
            RawPdfPolicy {
                scan_detector: Some("coverage".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(pdf.scan_detector, "coverage");
    }

    #[test]
    fn scan_detector_unknown_value_keeps_default() {
        let mut pdf = PdfPolicy::default();
        apply_pdf(
            &mut pdf,
            RawPdfPolicy {
                scan_detector: Some("nonsense".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(
            pdf.scan_detector, "aspect",
            "unknown detectors are rejected"
        );
    }

    #[test]
    fn scan_detector_from_toml_flows_through_raw_docs() {
        // The toml → Raw → apply path must carry scan_detector (the field was previously dropped
        // because the loader layers through Raw structs, not PdfPolicy directly).
        let raw: RawDocsPolicy =
            toml::from_str("[pdf]\nscan_detector = \"coverage\"\n").expect("parse");
        let mut docs = DocsPolicy::default();
        apply_docs(&mut docs, raw);
        assert_eq!(docs.pdf.scan_detector, "coverage");
    }
}
