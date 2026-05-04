use crate::errors::DocsError;

pub trait VlmDescriptionProvider {
    fn describe_image(&self, image_bytes: &[u8]) -> Result<String, DocsError>;
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
        policy.workers.vlm = "allow-local".into();
        let description = describe_image_with_policy(&policy, &MockVlm, b"image").unwrap();
        assert_eq!(description, "5 bytes described");
    }
}
