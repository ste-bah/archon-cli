use std::time::Duration;

use crate::errors::DocsError;
use crate::vlm;

const PDF_IMAGE_VLM_MAX_ATTEMPTS: usize = 3;

pub(crate) enum VlmImageResult {
    Described(vlm::VlmDescription),
    Disabled(String),
    NoProvider,
    Empty,
    Failed(String),
    Fatal(DocsError),
}

pub(crate) async fn describe_image(
    policy: archon_policy::EffectivePolicy,
    image_bytes: Vec<u8>,
) -> VlmImageResult {
    for attempt in 0..PDF_IMAGE_VLM_MAX_ATTEMPTS {
        let result = describe_image_once(policy.clone(), image_bytes.clone()).await;
        if attempt + 1 < PDF_IMAGE_VLM_MAX_ATTEMPTS && is_retryable(&result) {
            tokio::time::sleep(Duration::from_secs((attempt + 1) as u64)).await;
            continue;
        }
        return result;
    }
    unreachable!("PDF image VLM retry loop returns on every result")
}

async fn describe_image_once(
    policy: archon_policy::EffectivePolicy,
    image_bytes: Vec<u8>,
) -> VlmImageResult {
    let result = tokio::task::spawn_blocking(move || {
        vlm::describe_registered_image(&policy, &image_bytes, None)
    })
    .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            return VlmImageResult::Fatal(DocsError::VlmProvider {
                provider: "runtime".into(),
                message: format!("VLM worker join failed: {error}"),
                status_code: None,
            });
        }
    };
    match result {
        Err(error @ DocsError::VlmAuthentication { .. }) => VlmImageResult::Fatal(error),
        Err(
            error @ (DocsError::VlmProvider { .. }
            | DocsError::VlmRateLimit { .. }
            | DocsError::VlmTimeout { .. }),
        ) => VlmImageResult::Failed(format!("image description failed: {error}")),
        Err(error) => VlmImageResult::Failed(format!("image description failed: {error}")),
        Ok(vlm::VlmDescriptionOutcome::Disabled(reason)) => VlmImageResult::Disabled(reason),
        Ok(vlm::VlmDescriptionOutcome::NoProvider) => VlmImageResult::NoProvider,
        Ok(vlm::VlmDescriptionOutcome::Described(description))
            if description.text.trim().is_empty() =>
        {
            VlmImageResult::Empty
        }
        Ok(vlm::VlmDescriptionOutcome::Described(description)) => {
            VlmImageResult::Described(description)
        }
    }
}

fn is_retryable(result: &VlmImageResult) -> bool {
    match result {
        VlmImageResult::Empty => true,
        VlmImageResult::Failed(message) => {
            message.contains("did not contain text")
                || message.contains("provider returned empty")
                || message.contains("rate limit")
                || message.contains("timed out")
                || message.contains("status 5")
                || message.contains("HTTP 5")
        }
        _ => false,
    }
}
