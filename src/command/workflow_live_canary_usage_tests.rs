use std::process::Command as CanaryGitCommand;
use std::sync::{Arc, Mutex};

use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::create_default_registry;
use archon_core::subagent::SubagentManager;
use archon_core::subagent_executor::AgentSubagentExecutor;
use archon_learning::llm_call_usage::{LlmCallUsageScope, UsageAvailability, list_llm_call_usage};
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse as ProviderResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};
use archon_pipeline::llm_adapter::ProviderLlmAdapter;
use archon_pipeline::runner::LlmClient;
use archon_pipeline::subagent_adapter::SubagentPipelineClient;
use archon_tools::subagent_executor::install_subagent_executor;
use archon_tools::tool::ToolContext;
use archon_workflow::CommandAction;
use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use super::{CANARY_ARTIFACT_REL, CanaryAgentClient};
use crate::command::workflow_live::{LiveApprovalMode, run_live_action};

#[path = "workflow_live_canary_usage_tests_a.rs"]
mod workflow_live_canary_usage_tests_a;
use workflow_live_canary_usage_tests_a::*;
#[path = "workflow_live_canary_usage_tests_b.rs"]
mod workflow_live_canary_usage_tests_b;
use workflow_live_canary_usage_tests_b::*;
