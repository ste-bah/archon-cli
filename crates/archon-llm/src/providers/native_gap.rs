//! TASK-AGS-704: stub `LlmProvider` impls for the 4 gap-filler native
//! providers listed in TECH-AGS-PROVIDERS component `NativeProviderGap`
//! (lines 1141-1144): azure, cohere, copilot, minimax.
//!
//! Every method returns `LlmError::Unsupported` with the sentinel string
//! "Open Question #3" — preserved so `grep 'Open Question #3'
//! crates/archon-llm/src/providers/` finds all gap-fillers.
//!
//! Spec deviation (inherited from TASK-AGS-703 greenlit 2026-04-13):
//! spec says return `ProviderError::InvalidResponse` but the real trait
//! returns `LlmError`, so we surface the sentinel via
//! `LlmError::Unsupported` instead. Real wire impls are pending
//! stakeholder confirmation per spec line 1168.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use super::descriptor::ProviderDescriptor;
use crate::provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use crate::secrets::ApiKey;
use crate::streaming::StreamEvent;

fn gap_error(provider: &str) -> LlmError {
    LlmError::Unsupported(format!(
        "native provider {provider} not yet implemented — Open Question #3"
    ))
}

macro_rules! define_stub_provider {
    ($Type:ident, $name:literal) => {
        /// TASK-AGS-704 stub native provider — pending Open Question #3.
        pub struct $Type {
            pub(crate) descriptor: &'static ProviderDescriptor,
            #[allow(dead_code)]
            pub(crate) http: Arc<reqwest::Client>,
            #[allow(dead_code)]
            pub(crate) api_key: ApiKey,
        }

        impl $Type {
            pub fn new(
                descriptor: &'static ProviderDescriptor,
                http: Arc<reqwest::Client>,
                api_key: ApiKey,
            ) -> Self {
                Self {
                    descriptor,
                    http,
                    api_key,
                }
            }
        }

        #[async_trait]
        impl LlmProvider for $Type {
            fn name(&self) -> &str {
                &self.descriptor.display_name
            }

            fn models(&self) -> Vec<ModelInfo> {
                vec![ModelInfo {
                    id: self.descriptor.default_model.clone(),
                    display_name: self.descriptor.default_model.clone(),
                    context_window: 0,
                }]
            }

            async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
                Err(gap_error($name))
            }

            async fn stream(
                &self,
                _request: LlmRequest,
            ) -> Result<Receiver<StreamEvent>, LlmError> {
                Err(gap_error($name))
            }

            fn supports_feature(&self, _feature: ProviderFeature) -> bool {
                false
            }
        }
    };
}

define_stub_provider!(AzureProvider, "azure");
define_stub_provider!(CohereProvider, "cohere");
define_stub_provider!(CopilotProvider, "copilot");
define_stub_provider!(MinimaxProvider, "minimax");
