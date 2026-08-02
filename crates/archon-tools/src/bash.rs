use std::path::PathBuf;

use serde_json::json;
use std::sync::LazyLock;

use crate::provider_env::{ProviderEnvPolicy, ProviderEnvResolution, ProviderEnvSource};
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

#[path = "bash_output.rs"]
mod bash_output;

#[path = "bash_process.rs"]
mod bash_process;
use bash_process::{
    command_from_input, execute_in_sandbox, limit_tool_result, prepare_command,
    run_prepared_bash_command,
};

#[cfg(test)]
#[path = "bash_process_tests.rs"]
mod bash_process_tests;

#[path = "bash_env.rs"]
mod bash_env;
pub use bash_env::sanitized_env;

const DEFAULT_BASH_TIMEOUT_SECS: u64 = 600;

static BASH_PROGRAM: LazyLock<PathBuf> =
    LazyLock::new(|| select_bash_program(which::which("bash").ok(), which::which("bash.exe").ok()));

fn select_bash_program(bash: Option<PathBuf>, bash_exe: Option<PathBuf>) -> PathBuf {
    bash.or(bash_exe).unwrap_or_else(|| PathBuf::from("bash"))
}

const BASH_COMPAT_PRELUDE: &str = r#"
printf() {
    if [ "${1-}" = "-v" ]; then
        builtin printf "$@"
        return
    fi
    if [ "${1-}" = "--" ]; then
        shift
        builtin printf -- "$@"
        return
    fi
    builtin printf -- "$@"
}
"#;

const SHELL_TIMEOUT_PRELUDE: &str = r#"
timeout() {
    while [ "$#" -gt 0 ]; do
        case "${1-}" in
            --) shift; break ;;
            -k|--kill-after|-s|--signal)
                shift
                if [ "$#" -gt 0 ]; then shift; fi
                ;;
            --foreground|--preserve-status|-v|--verbose)
                shift
                ;;
            -*)
                shift
                ;;
            *[0-9]s|*[0-9]m|*[0-9]h|*[0-9]d|[0-9]*|[0-9]*.*)
                shift
                break
                ;;
            *)
                break
                ;;
        esac
    done
    if [ "$#" -eq 0 ]; then
        return 125
    fi
    "$@"
}

gtimeout() {
    timeout "$@"
}
"#;

#[derive(Clone)]
pub struct BashTool {
    pub timeout_secs: u64,
    pub max_output_bytes: usize,
    pub safe_commands: Vec<String>,
    pub risky_commands: Vec<String>,
    pub dangerous_commands: Vec<String>,
    pub provider_env: Option<ProviderEnvSource>,
}

impl Default for BashTool {
    fn default() -> Self {
        Self {
            timeout_secs: DEFAULT_BASH_TIMEOUT_SECS,
            max_output_bytes: 102400,
            safe_commands: Vec::new(),
            risky_commands: Vec::new(),
            dangerous_commands: Vec::new(),
            provider_env: None,
        }
    }
}

impl BashTool {
    pub fn with_provider_env(mut self, provider_env: ProviderEnvPolicy) -> Self {
        self.provider_env = Some(ProviderEnvSource::Policy(provider_env));
        self
    }

    pub fn with_provider_env_resolution(mut self, provider_env: ProviderEnvResolution) -> Self {
        self.provider_env = Some(ProviderEnvSource::Resolution(provider_env));
        self
    }

    pub fn with_provider_env_source(mut self, provider_env: ProviderEnvSource) -> Self {
        self.provider_env = Some(provider_env);
        self
    }
}

#[async_trait::async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "Bash"
    }

    fn description(&self) -> &str {
        "Executes a bash command and returns its output."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Optional timeout in milliseconds. Values below the configured tools.bash_timeout shorten this command; larger values are clamped to that configured maximum."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let raw_command = match command_from_input(&input) {
            Ok(command) => command,
            Err(error) => return limit_tool_result(self.max_output_bytes, error),
        };
        let timeout_ms = effective_timeout_ms(
            input.get("timeout").and_then(|value| value.as_u64()),
            self.timeout_secs * 1000,
        );
        let prepared = match prepare_command(self, raw_command, timeout_ms, ctx).await {
            Ok(prepared) => prepared,
            Err(error) => return limit_tool_result(self.max_output_bytes, error),
        };
        if let Some(result) = execute_in_sandbox(self, ctx, &prepared).await {
            return result;
        }
        run_prepared_bash_command(self, ctx, raw_command, prepared).await
    }

    fn permission_level(&self, input: &serde_json::Value) -> PermissionLevel {
        let command = input.get("command").and_then(|v| v.as_str()).unwrap_or("");

        match archon_permissions::classifier::classify_command(
            command,
            &self.safe_commands,
            &self.risky_commands,
            &self.dangerous_commands,
        ) {
            archon_permissions::classifier::CommandClass::Safe => PermissionLevel::Safe,
            archon_permissions::classifier::CommandClass::Risky => PermissionLevel::Risky,
            archon_permissions::classifier::CommandClass::Dangerous => PermissionLevel::Dangerous,
        }
    }

    fn with_provider_env_source(&self, provider_env: ProviderEnvSource) -> Option<Box<dyn Tool>> {
        Some(Box::new(
            self.clone().with_provider_env_source(provider_env),
        ))
    }
}

async fn provider_env_overlay(source: Option<&ProviderEnvSource>) -> Option<ProviderEnvResolution> {
    let source = source?;
    if let Some(resolved) = source.resolution()
        && source.policy().is_none_or(|policy| resolved.covers(policy))
    {
        return Some(resolved.clone());
    }
    let policy = source.policy()?;
    Some(crate::provider_env::resolve_provider_env(policy).await)
}

fn redact_provider_env_output(
    provider_env: Option<&ProviderEnvResolution>,
    output: String,
) -> String {
    provider_env.map_or(output.clone(), |env| env.redact_text(&output))
}

fn effective_timeout_ms(requested_ms: Option<u64>, configured_ms: u64) -> u64 {
    requested_ms.unwrap_or(configured_ms).min(configured_ms)
}

fn command_with_compat_prelude(command: &str) -> String {
    format!("{BASH_COMPAT_PRELUDE}\n{SHELL_TIMEOUT_PRELUDE}\n{command}")
}

#[cfg(test)]
#[path = "bash_tests.rs"]
mod tests;
