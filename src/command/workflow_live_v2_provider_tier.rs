//! The provider tier a V2 agent request runs on: its role, else its method.
use super::*;

pub(crate) fn provider_tier_for_v2_request(
    request: &archon_workflow::WorkflowV2AgentRequest,
) -> ProviderTier {
    match request.role.to_ascii_lowercase().as_str() {
        "planner" => ProviderTier::Planner,
        "researcher" => ProviderTier::Researcher,
        "coder" | "implementation" => ProviderTier::Coder,
        "critic" => ProviderTier::Critic,
        "reducer" => ProviderTier::Reducer,
        "cheap" => ProviderTier::Cheap,
        "local" | "tool" => ProviderTier::Local,
        "vision" => ProviderTier::Vision,
        _ => match request.call.method {
            WorkflowV2HostMethod::Implementation => ProviderTier::Coder,
            WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => {
                ProviderTier::Reducer
            }
            WorkflowV2HostMethod::QualityGate | WorkflowV2HostMethod::HumanGate => {
                ProviderTier::Critic
            }
            WorkflowV2HostMethod::Tool
            | WorkflowV2HostMethod::HostCommand
            | WorkflowV2HostMethod::SaveArtifact
            | WorkflowV2HostMethod::RequireArtifact
            | WorkflowV2HostMethod::Checkpoint => ProviderTier::Local,
            WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => ProviderTier::Coder,
            WorkflowV2HostMethod::Agent => ProviderTier::Researcher,
        },
    }
}
