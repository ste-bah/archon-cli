#!/usr/bin/env python3
"""Exercise dependency-light class C production modules without Cargo.

The binding harness substitutes configuration/registry data types only; it includes
production selection, capture and binding code unchanged. Full crate tests remain
necessary when build slots are available.
"""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def run_rust(name, source):
    with tempfile.TemporaryDirectory(prefix=f"class-c-{name}-") as directory:
        source_path = Path(directory) / "test.rs"
        binary = Path(directory) / "tests"
        source_path.write_text(source)
        subprocess.run(["rustc", "--edition=2024", "--test", str(source_path),
                        "-o", str(binary)], check=True)
        subprocess.run([str(binary)], check=True)


def path(relative):
    return '"' + str(ROOT / relative) + '"'


run_rust("policy", f'#[path = {path("crates/archon-core/src/config/model_policy.rs")}] mod policy;')

run_rust("bindings", '''
#![allow(dead_code)]
extern crate self as archon_core;
extern crate self as archon_llm;
pub mod config {
    use std::collections::BTreeMap;
    #[derive(Clone, Default)]
    pub struct ProviderEnvironmentConfig {
        pub base_url: Option<String>, pub base_url_env: Option<String>,
        pub api_key_env: Option<String>, pub credential_env: Vec<String>,
    }
    #[derive(Clone, Default)]
    pub struct LlmConfig {
        pub provider: String,
        pub provider_environment: BTreeMap<String, ProviderEnvironmentConfig>,
        pub openai: OpenAi, pub local: Local,
    }
    #[derive(Clone, Default)]
    pub struct OpenAi { pub base_url: Option<String>, pub api_key: Option<String> }
    #[derive(Clone, Default)]
    pub struct Local { pub base_url: String }
    #[derive(Default)] pub struct Api { pub base_url: Option<String> }
    #[derive(Default)] pub struct Codex { pub app_server_url: Option<String> }
    #[derive(Default)] pub struct Providers { pub openai_codex: Codex }
    #[derive(Default)] pub struct ArchonConfig { pub llm: LlmConfig, pub api: Api, pub providers: Providers }
''' + f'#[path = {path("crates/archon-core/src/config/model_policy.rs")}] pub mod model_policy;\n}}' + '''
pub mod env_vars {
    #[derive(Default)]
    pub struct ArchonEnvVars {
        pub anthropic_api_key: Option<String>, pub archon_api_key: Option<String>,
        pub archon_oauth_token: Option<String>, pub model: Option<String>,
        pub effort: Option<String>, pub config_dir: Option<std::path::PathBuf>,
    }
}
pub mod providers { pub mod registry {
    pub struct Descriptor { pub id: String, pub base_url: String, pub env_key_var: String }
    pub fn list_compat() -> Vec<Descriptor> {
        vec![Descriptor { id: "groq".into(), base_url: "https://registry.invalid".into(), env_key_var: "GROQ_API_KEY".into() }]
    }
} }
mod command {
    pub mod workflow_provider_route {
        #[derive(PartialEq, Eq, Clone, Copy)] pub enum ProviderEndpointPolicy { AmbientAllowed, ConfiguredOnly }
''' + f'#[path = {path("src/command/workflow_provider_bindings.rs")}] mod bindings_impl;\n' + '''
        pub(crate) use bindings_impl::{bindings, runtime_config};
    }
    mod decompose {
''' + f'#[path = {path("src/command/workflow_decompose_provider_environment.rs")}] pub mod environment;\n' + '''
    }
    #[test]
    fn configured_credentials_and_runtime_endpoints_share_bindings() {
        use crate::config::*;
        use workflow_provider_route::*;
        let mut config = ArchonConfig::default();
        config.llm.provider = "private-service".into();
        config.llm.provider_environment.insert("private-service".into(), ProviderEnvironmentConfig {
            base_url: Some("configured".into()), base_url_env: Some("CLASS_C_URL".into()),
            api_key_env: Some("CLASS_C_KEY".into()), credential_env: vec!["CLASS_C_TOKEN".into()],
        });
        unsafe {
            std::env::set_var("CLASS_C_URL", "ambient");
            std::env::set_var("CLASS_C_KEY", "secret");
            std::env::set_var("CLASS_C_TOKEN", "token");
        }
        let configured = runtime_config(&config, ProviderEndpointPolicy::ConfiguredOnly);
        assert_eq!(configured.provider_environment["private-service"].base_url.as_deref(), Some("configured"));
        let ambient = runtime_config(&config, ProviderEndpointPolicy::AmbientAllowed);
        assert_eq!(ambient.provider_environment["private-service"].base_url.as_deref(), Some("ambient"));
        let frozen = decompose::environment::freeze_provider_environment(&config, &Default::default());
        assert_eq!(frozen["CLASS_C_URL"], "ambient");
        assert_eq!(frozen["CLASS_C_KEY"], "secret");
        assert_eq!(frozen["CLASS_C_TOKEN"], "token");
    }
}
''')

# Compile the real tier resolver with minimal synchronous provider-port types.
# Only serialization annotations are removed; resolver bodies are unchanged.
tiers = (ROOT / "crates/archon-workflow/src/provider_tiers.rs").read_text()
tiers = tiers.replace("use serde::{Deserialize, Serialize};\n", "")
tiers = tiers.replace(", Serialize, Deserialize", "")
tiers = tiers.replace('#[serde(rename_all = "snake_case")]\n', "")
run_rust("tiers", '''
#![allow(dead_code)]
extern crate self as archon_llm;
mod spec {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub enum ProviderTier { Cheap, Local, Critic, Reducer, Planner, Researcher, Coder, Vision }
}
mod error {
    #[derive(Debug)] pub enum WorkflowError { ProviderTierUnresolved(String) }
    pub type WorkflowResult<T> = Result<T, WorkflowError>;
}
mod config {
    use crate::spec::ProviderTier;
    #[derive(Default)] pub struct WorkflowConfig { pub provider_tiers: std::collections::BTreeMap<ProviderTier, String> }
    pub fn default_provider_tiers() -> std::collections::BTreeMap<ProviderTier, String> { Default::default() }
}
pub mod provider {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum DataFlowClassification { Local, UserOperated, Cloud }
    #[derive(Clone, Copy)] pub enum ProviderFeature { Vision, Thinking, ToolUse, PromptCaching, SystemPrompt, Streaming }
    #[derive(Default)] pub struct LlmRequest { pub model: String }
    pub trait LlmProvider {
        fn name(&self) -> &str;
        fn data_flow_classification(&self) -> DataFlowClassification;
        fn supports_feature(&self, feature: ProviderFeature) -> bool;
        fn resolve_request_model(&self, request: &mut LlmRequest);
    }
    #[derive(Default)] pub struct ProviderRegistry { pub provider: Option<Box<dyn LlmProvider>> }
    impl ProviderRegistry {
        pub fn active(&self, name: &str) -> Result<&dyn LlmProvider, String> {
            self.provider.as_deref().filter(|p| p.name() == name).ok_or_else(|| "unregistered".into())
        }
    }
}
mod tiers {
''' + tiers + '''
    #[cfg(test)] mod tests {
        use super::*;
        use archon_llm::provider::LlmProvider;
        struct Custom { flow: DataFlowClassification, vision: bool }
        impl LlmProvider for Custom {
            fn name(&self) -> &str { "arbitrary-name" }
            fn data_flow_classification(&self) -> DataFlowClassification { self.flow }
            fn supports_feature(&self, feature: ProviderFeature) -> bool { matches!(feature, ProviderFeature::Vision) && self.vision }
            fn resolve_request_model(&self, request: &mut LlmRequest) { request.model = format!("resolved-{}", request.model); }
        }
        #[test] fn registered_names_and_capabilities_drive_resolution() {
            for flow in [DataFlowClassification::Cloud, DataFlowClassification::Local, DataFlowClassification::UserOperated] {
                let registry = ProviderRegistry { provider: Some(Box::new(Custom { flow, vision: true })) };
                let resolver = ProviderTierResolver::new("arbitrary-name", "ignored");
                let result = resolver.resolve_with_registry(ProviderTier::Planner, &registry).unwrap();
                assert_eq!(result.model, "resolved-sonnet");
                assert_eq!(result.local, flow == DataFlowClassification::Local);
                assert_eq!(resolver.resolve_with_registry(ProviderTier::Local, &registry).is_ok(), flow == DataFlowClassification::Local);
                assert_eq!(result.provider_family, match flow {
                    DataFlowClassification::Local => ProviderFamily::Local,
                    DataFlowClassification::Cloud => ProviderFamily::Cloud,
                    DataFlowClassification::UserOperated => ProviderFamily::UserOperated,
                });
            }
        }
        #[test] fn unavailable_metadata_and_capabilities_fail_explicitly() {
            let resolver = ProviderTierResolver::new("arbitrary-name", "model");
            assert!(resolver.resolve(ProviderTier::Planner).is_err());
            assert!(classify_provider("local-name").is_err());
            assert!(resolver.resolve_with_registry(ProviderTier::Planner, &ProviderRegistry::default()).is_err());
            let registry = ProviderRegistry { provider: Some(Box::new(Custom { flow: DataFlowClassification::Cloud, vision: false })) };
            assert!(resolver.resolve_with_registry(ProviderTier::Vision, &registry).is_err());
        }
        #[test] fn configured_tiers_still_resolve_at_provider_boundary() {
            let config = WorkflowConfig { provider_tiers: BTreeMap::from([(ProviderTier::Planner, "haiku".into())]) };
            let registry = ProviderRegistry { provider: Some(Box::new(Custom { flow: DataFlowClassification::Cloud, vision: true })) };
            let result = ProviderTierResolver::new("arbitrary-name", "ignored").with_config(&config)
                .resolve_with_registry(ProviderTier::Planner, &registry).unwrap();
            assert_eq!(result.model, "resolved-haiku");
        }
    }
}
''')

adapter = (ROOT / "crates/archon-pipeline/src/llm_adapter.rs").read_text()
start = adapter.index("    fn model_for_provider(")
end = adapter.index("\n}\n", start)
method = adapter[start:end]
run_rust("model-selection", '''
extern crate self as archon_core;
mod config {
''' + f'#[path = {path("crates/archon-core/src/config/model_policy.rs")}] pub mod model_policy;\n}}' + '''
#[derive(Default)] struct LlmRequest { model: String }
struct Provider;
impl Provider {
    fn resolve_request_model(&self, request: &mut LlmRequest) {
        if ["haiku", "sonnet", "opus"].contains(&request.model.as_str()) {
            request.model = format!("custom-{}", request.model);
        }
    }
}
struct Adapter { provider: Provider, tier_aliases: std::collections::BTreeMap<String, String> }
impl Adapter {
''' + method + '''
}
#[test] fn explicit_model_is_not_replaced_by_another_provider_model() {
    let adapter = Adapter { provider: Provider, tier_aliases: Default::default() };
    assert_eq!(adapter.model_for_provider("claude-explicit"), "claude-explicit");
    assert_eq!(adapter.model_for_provider("custom-explicit"), "custom-explicit");
}
#[test] fn configurable_aliases_are_delegated_to_provider() {
    let adapter = Adapter { provider: Provider, tier_aliases: std::collections::BTreeMap::from([("sonnet".into(), "haiku".into())]) };
    assert_eq!(adapter.model_for_provider("sonnet"), "custom-haiku");
}
''')
