use crate::errors::DocsError;
use std::sync::{Arc, RwLock};

#[path = "vlm/anthropic.rs"]
pub mod anthropic;
#[path = "vlm/factory.rs"]
pub mod factory;
#[path = "vlm/gemini.rs"]
pub mod gemini;
#[path = "vlm/mime.rs"]
pub mod mime;
#[path = "vlm/ollama.rs"]
pub mod ollama;

pub trait VlmDescriptionProvider: Send + Sync {
    fn describe_image(&self, image_bytes: &[u8]) -> Result<String, DocsError>;
}

pub const IMAGE_DESCRIPTION_PROMPT: &str = r#"You are a precise image describer. Describe what the image shows in 2-4 sentences.
For charts, name the chart type, axes labels, time period, observable patterns, and any annotations.
For diagrams, describe the structure and labelled components.
For photos, describe subject + setting + relevant detail.
Write factually. Do not speculate. Do not add framing language ("This image shows...")."#;

#[derive(Clone, Debug, PartialEq)]
pub struct VlmProviderMetadata {
    pub provider: String,
    pub model: String,
    pub cost_usd: f64,
}

impl VlmProviderMetadata {
    pub fn new(provider: impl Into<String>, model: impl Into<String>, cost_usd: f64) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            cost_usd,
        }
    }
}

#[derive(Clone)]
pub struct RegisteredVlmProvider {
    metadata: VlmProviderMetadata,
    provider: Arc<dyn VlmDescriptionProvider>,
}

impl RegisteredVlmProvider {
    pub fn new(metadata: VlmProviderMetadata, provider: Box<dyn VlmDescriptionProvider>) -> Self {
        Self {
            metadata,
            provider: Arc::from(provider),
        }
    }

    pub fn metadata(&self) -> &VlmProviderMetadata {
        &self.metadata
    }

    pub fn provider(&self) -> Arc<dyn VlmDescriptionProvider> {
        Arc::clone(&self.provider)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VlmDescription {
    pub text: String,
    pub provider: String,
    pub model: String,
    pub cost_usd: f64,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VlmDescriptionOutcome {
    Disabled(String),
    NoProvider,
    Described(VlmDescription),
}

pub fn describe_image_with_policy(
    policy: &archon_policy::EffectivePolicy,
    provider: &dyn VlmDescriptionProvider,
    image_bytes: &[u8],
) -> Result<String, DocsError> {
    let decision = policy.docs_vlm_decision();
    if !decision.allowed {
        return Err(DocsError::VlmPolicyDenied {
            message: decision.reason,
        });
    }
    provider.describe_image(image_bytes)
}

static PROVIDER: RwLock<Option<Arc<RegisteredVlmProvider>>> = RwLock::new(None);

pub fn get_provider() -> Option<Arc<dyn VlmDescriptionProvider>> {
    PROVIDER
        .read()
        .ok()
        .and_then(|guard| guard.as_ref().map(|registered| registered.provider()))
}

pub fn get_registered_provider() -> Option<Arc<RegisteredVlmProvider>> {
    PROVIDER.read().ok().and_then(|guard| guard.clone())
}

pub fn set_provider(provider: Box<dyn VlmDescriptionProvider>) {
    let registered =
        RegisteredVlmProvider::new(VlmProviderMetadata::new("test", "mock-vlm", 0.0), provider);
    set_registered_provider(registered);
}

pub fn set_registered_provider(provider: RegisteredVlmProvider) {
    if let Ok(mut guard) = PROVIDER.write() {
        *guard = Some(Arc::new(provider));
    }
}

pub fn clear_provider() {
    if let Ok(mut guard) = PROVIDER.write() {
        *guard = None;
    }
}

pub fn describe_registered_image(
    policy: &archon_policy::EffectivePolicy,
    image_bytes: &[u8],
) -> Result<VlmDescriptionOutcome, DocsError> {
    let decision = policy.docs_vlm_decision();
    if !decision.allowed {
        return Ok(VlmDescriptionOutcome::Disabled(decision.reason));
    }
    let Some(registered) = get_registered_provider() else {
        return Ok(VlmDescriptionOutcome::NoProvider);
    };
    let started = std::time::Instant::now();
    let text = registered.provider.describe_image(image_bytes)?;
    let metadata = registered.metadata();
    Ok(VlmDescriptionOutcome::Described(VlmDescription {
        text,
        provider: metadata.provider.clone(),
        model: metadata.model.clone(),
        cost_usd: metadata.cost_usd,
        duration_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockVlm;

    impl VlmDescriptionProvider for MockVlm {
        fn describe_image(&self, image_bytes: &[u8]) -> Result<String, DocsError> {
            Ok(format!("{} bytes described", image_bytes.len()))
        }
    }

    #[test]
    fn vlm_denied_by_default_policy() {
        let policy = archon_policy::EffectivePolicy::default();
        let err = describe_image_with_policy(&policy, &MockVlm, b"image").unwrap_err();
        assert!(matches!(err, DocsError::VlmPolicyDenied { .. }));
    }

    #[test]
    fn vlm_allowed_when_local_policy_enabled() {
        let mut policy = archon_policy::EffectivePolicy::default();
        policy.docs.vlm.enabled = true;
        policy.docs.vlm.mode = "local".into();
        policy.docs.vlm.provider = "ollama".into();
        policy.workers.vlm = "allow-local".into();
        let description = describe_image_with_policy(&policy, &MockVlm, b"image").unwrap();
        assert_eq!(description, "5 bytes described");
    }
}
