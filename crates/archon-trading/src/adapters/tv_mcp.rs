use crate::agent_policy::{EscalationTrigger, Persona, escalation_for};
use crate::maker_checker::{MakerCheckerApproval, MakerCheckerError};
use archon_mcp::types::{McpToolResult, ToolContent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

const MAX_RETRIES: u8 = 3;
const COMPILE_SLA_MS: u128 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TvMcpConfig {
    pub adapter_pin: String,
    pub sandbox_certified: bool,
    pub write_tier_enabled: bool,
}

impl TvMcpConfig {
    pub fn pinned(adapter_pin: impl Into<String>) -> Self {
        Self {
            adapter_pin: adapter_pin.into(),
            sandbox_certified: false,
            write_tier_enabled: false,
        }
    }

    pub fn validate_pin(&self) -> Result<(), TvMcpError> {
        let (vendor, sha) = self
            .adapter_pin
            .split_once('@')
            .ok_or(TvMcpError::MissingAdapterPin)?;
        if vendor.trim().is_empty() || sha.len() < 7 || !sha.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(TvMcpError::MissingAdapterPin);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TvReadAction {
    DocsLookup,
    PineCompileCheck,
    ScreenshotCapture,
    ScriptVersionSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TvWriteAction {
    ChartDeploy,
    AlertSetup,
    TerminalInteraction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TvMcpResponse {
    pub content_text: Vec<String>,
    pub attempts: u8,
    pub elapsed_ms: u128,
    pub adapter_pin: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TvMcpError {
    MissingAdapterPin,
    WriteTierDenied {
        reason: &'static str,
        escalate_to: Persona,
    },
    MakerChecker(MakerCheckerError),
    McpFailureEscalated {
        attempts: u8,
        partial_script_persisted: bool,
    },
    CompileSlaExceeded {
        elapsed_ms: u128,
    },
}

pub trait TvMcpTransport {
    fn call_tool(&mut self, tool_name: &str, arguments: Value) -> Result<TimedMcpResult, String>;
}

#[derive(Debug, Clone)]
pub struct TimedMcpResult {
    pub result: McpToolResult,
    pub elapsed: Duration,
}

#[derive(Debug, Clone)]
pub struct TradingViewMcpAdapter {
    config: TvMcpConfig,
}

impl TradingViewMcpAdapter {
    pub fn new(config: TvMcpConfig) -> Result<Self, TvMcpError> {
        config.validate_pin()?;
        Ok(Self { config })
    }

    pub fn docs_lookup<T: TvMcpTransport>(
        &self,
        transport: &mut T,
        query: &str,
    ) -> Result<TvMcpResponse, TvMcpError> {
        self.read_call(
            transport,
            TvReadAction::DocsLookup,
            json!({ "query": query }),
        )
    }

    pub fn pine_compile_check<T: TvMcpTransport>(
        &self,
        transport: &mut T,
        script: &str,
    ) -> Result<TvMcpResponse, TvMcpError> {
        let response = self.read_call(
            transport,
            TvReadAction::PineCompileCheck,
            json!({ "script": script, "pine_version": "v6" }),
        )?;
        if response.elapsed_ms > COMPILE_SLA_MS {
            return Err(TvMcpError::CompileSlaExceeded {
                elapsed_ms: response.elapsed_ms,
            });
        }
        Ok(response)
    }

    pub fn screenshot_capture<T: TvMcpTransport>(
        &self,
        transport: &mut T,
        chart_id: &str,
    ) -> Result<TvMcpResponse, TvMcpError> {
        self.read_call(
            transport,
            TvReadAction::ScreenshotCapture,
            json!({ "chart_id": chart_id }),
        )
    }

    pub fn script_version_sync<T: TvMcpTransport>(
        &self,
        transport: &mut T,
        script_id: &str,
    ) -> Result<TvMcpResponse, TvMcpError> {
        self.read_call(
            transport,
            TvReadAction::ScriptVersionSync,
            json!({ "script_id": script_id }),
        )
    }

    pub fn write_action<T: TvMcpTransport>(
        &self,
        transport: &mut T,
        action: TvWriteAction,
        arguments: Value,
        approval: Option<&MakerCheckerApproval>,
    ) -> Result<TvMcpResponse, TvMcpError> {
        self.ensure_write_allowed(approval)?;
        self.call_with_fail_closed_retries(transport, write_tool(action), arguments)
    }

    pub const fn config(&self) -> &TvMcpConfig {
        &self.config
    }

    fn read_call<T: TvMcpTransport>(
        &self,
        transport: &mut T,
        action: TvReadAction,
        arguments: Value,
    ) -> Result<TvMcpResponse, TvMcpError> {
        self.call_with_fail_closed_retries(transport, read_tool(action), arguments)
    }

    fn ensure_write_allowed(
        &self,
        approval: Option<&MakerCheckerApproval>,
    ) -> Result<(), TvMcpError> {
        if !self.config.write_tier_enabled {
            return Err(write_denied("write tier disabled"));
        }
        if !self.config.sandbox_certified {
            return Err(write_denied("sandbox certification required"));
        }
        approval
            .ok_or_else(|| write_denied("maker-checker approval required"))?
            .verify_pair()
            .map_err(TvMcpError::MakerChecker)
    }

    fn call_with_fail_closed_retries<T: TvMcpTransport>(
        &self,
        transport: &mut T,
        tool_name: &str,
        arguments: Value,
    ) -> Result<TvMcpResponse, TvMcpError> {
        for attempt in 1..=MAX_RETRIES {
            match transport.call_tool(tool_name, arguments.clone()) {
                Ok(timed) if !timed.result.is_error => {
                    return Ok(self.response_from(timed, attempt));
                }
                Ok(_) | Err(_) => {
                    let _delay = retry_backoff(attempt);
                }
            }
        }
        Err(TvMcpError::McpFailureEscalated {
            attempts: MAX_RETRIES,
            partial_script_persisted: false,
        })
    }

    fn response_from(&self, timed: TimedMcpResult, attempts: u8) -> TvMcpResponse {
        TvMcpResponse {
            content_text: content_text(timed.result.content),
            attempts,
            elapsed_ms: timed.elapsed.as_millis(),
            adapter_pin: self.config.adapter_pin.clone(),
        }
    }
}

fn write_denied(reason: &'static str) -> TvMcpError {
    let decision = escalation_for(EscalationTrigger::UncertifiedMcpWrite);
    TvMcpError::WriteTierDenied {
        reason,
        escalate_to: decision.escalate_to.unwrap_or(Persona::Per01HumanGovernor),
    }
}

const fn read_tool(action: TvReadAction) -> &'static str {
    match action {
        TvReadAction::DocsLookup => "tv.docs_lookup",
        TvReadAction::PineCompileCheck => "tv.pine_compile_check",
        TvReadAction::ScreenshotCapture => "tv.screenshot_capture",
        TvReadAction::ScriptVersionSync => "tv.script_version_sync",
    }
}

const fn write_tool(action: TvWriteAction) -> &'static str {
    match action {
        TvWriteAction::ChartDeploy => "tv.chart_deploy",
        TvWriteAction::AlertSetup => "tv.alert_setup",
        TvWriteAction::TerminalInteraction => "tv.terminal_interaction",
    }
}

fn content_text(content: Vec<ToolContent>) -> Vec<String> {
    content
        .into_iter()
        .filter_map(|item| match item {
            ToolContent::Text { text } => Some(text),
            ToolContent::Resource { text, .. } => text,
            ToolContent::Image { .. } => Some("<image>".to_string()),
        })
        .collect()
}

fn retry_backoff(attempt: u8) -> Duration {
    Duration::from_millis(100_u64.saturating_mul(1_u64 << attempt.saturating_sub(1)))
}

#[cfg(test)]
#[path = "tv_mcp_tests.rs"]
mod tests;
