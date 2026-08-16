use std::path::PathBuf;

use serde_json::json;
use std::sync::LazyLock;

use crate::provider_env::{ProviderEnvPolicy, ProviderEnvResolution, ProviderEnvSource};
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult, WorkingTreeEffect};
use crate::workflow_resource_env::CargoResourceLimits;

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
pub(crate) mod bash_env;
pub use bash_env::sanitized_env;

const DEFAULT_BASH_TIMEOUT_SECS: u64 = 3600;

/// Default floor, in seconds, under which a caller-supplied `timeout` cannot
/// drag a command. Mirrors `tools.bash_timeout_floor`.
const DEFAULT_BASH_TIMEOUT_FLOOR_SECS: u64 = 1800;

/// The bash this tool runs commands with.
///
/// Delegated to `archon-shell` rather than resolved here. This was
/// `which("bash").or(which("bash.exe"))`, a bare PATH lookup with no Windows
/// rules at all, so on a default Git for Windows install — where Git's `bin` is
/// not added to PATH — it selected `C:\Windows\System32\bash.exe`, the WSL
/// launcher. That does not fail on a Windows path; it runs the command inside a
/// Linux filesystem namespace that cannot see the working directory, so every
/// command came back with empty output and success. Eleven tests in this crate
/// failed on exactly that, and it is #118 as reported.
///
/// `archon-shell` already had the two rules that fix it — prefer the bash beside
/// `git.exe`, never accept the WSL launcher — and had them under test. Calling
/// its resolver keeps one implementation rather than a second copy that can
/// drift.
static BASH_PROGRAM: LazyLock<PathBuf> =
    LazyLock::new(|| archon_shell::resolve_bash().to_path_buf());

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
    /// Floor on the caller-supplied `timeout`, from `tools.bash_timeout_floor`.
    pub timeout_floor_secs: u64,
    pub max_output_bytes: usize,
    pub safe_commands: Vec<String>,
    pub risky_commands: Vec<String>,
    pub dangerous_commands: Vec<String>,
    pub provider_env: Option<ProviderEnvSource>,
    /// Resource defaults for agent-run `cargo` commands, from `[tools.cargo]`.
    pub cargo_limits: CargoResourceLimits,
    /// How isolated the agent owning this tool is (#184 M3).
    ///
    /// Carried on the tool rather than on `ToolContext` because the registry is
    /// already built per subagent, and because the two existing gates cannot be
    /// relied on: `tool_run_admission` is inherited verbatim from the parent and
    /// is skipped entirely for `PermissionLevel::Safe` — which a user's own
    /// `safe_commands` entry for `cargo` would trigger — and `sandbox` is `None`
    /// unless the operator turned it on.
    pub isolation_tier: crate::isolation::IsolationTier,
}

impl Default for BashTool {
    fn default() -> Self {
        Self {
            timeout_secs: DEFAULT_BASH_TIMEOUT_SECS,
            timeout_floor_secs: DEFAULT_BASH_TIMEOUT_FLOOR_SECS,
            max_output_bytes: 102400,
            safe_commands: Vec::new(),
            risky_commands: Vec::new(),
            dangerous_commands: Vec::new(),
            provider_env: None,
            cargo_limits: CargoResourceLimits::default(),
            // The main agent and any non-isolated subagent: unrestricted.
            isolation_tier: crate::isolation::IsolationTier::Shared,
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

    /// Restrict this tool to what its agent's isolation tier permits (#184 M3).
    pub fn with_isolation_tier(mut self, tier: crate::isolation::IsolationTier) -> Self {
        self.isolation_tier = tier;
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
                    "description": "Optional timeout in milliseconds, clamped to the configured tools.bash_timeout_floor..tools.bash_timeout range. Shortening below the floor has no effect, so do not use this to make a build or test run fail fast."
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

        // Refuse before anything is prepared or spawned: at
        // `IsolationTier::Worktree` a single `cargo check` creates the cold
        // `target/` the tier exists to avoid, and there is no undoing it
        // afterwards (#184 M3).
        if !self.isolation_tier.may_build()
            && let Some(segment) = crate::isolation::build_command_in(raw_command)
        {
            return limit_tool_result(
                self.max_output_bytes,
                ToolResult::error(crate::isolation::build_refusal(&segment)),
            );
        }

        let timeout_ms = effective_timeout_ms(
            input.get("timeout").and_then(|value| value.as_u64()),
            self.timeout_secs * 1000,
            self.timeout_floor_secs * 1000,
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

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::Arbitrary
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

    fn with_isolation_tier(&self, tier: crate::isolation::IsolationTier) -> Option<Box<dyn Tool>> {
        Some(Box::new(self.clone().with_isolation_tier(tier)))
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

/// Resolve the timeout for one command from the caller's request and the
/// configured bounds.
///
/// The caller's `timeout` used to be a ceiling with no floor —
/// `requested.unwrap_or(configured).min(configured)` — so a model could shorten
/// any command to an arbitrarily small value. That argument is model-supplied
/// and a model has no way to know how long a cold build of a large Rust
/// workspace takes, so a guessed two minutes killed work that `bash_timeout` had
/// budgeted an hour for. Nothing in the failure told anyone the model had chosen
/// the limit; it read as an ordinary timeout.
///
/// `floor_ms` is clamped to `configured_ms` first, so an operator who
/// deliberately sets a short `bash_timeout` still gets it. When the floor meets
/// or exceeds the ceiling the request is pinned to the ceiling — the argument
/// stops having an effect, which is the correct reading of a configuration that
/// leaves it no room.
fn effective_timeout_ms(requested_ms: Option<u64>, configured_ms: u64, floor_ms: u64) -> u64 {
    let Some(requested) = requested_ms else {
        return configured_ms;
    };
    // `clamp` panics when min > max, so the floor is bounded by the ceiling
    // before use rather than trusted to be ordered.
    requested.clamp(floor_ms.min(configured_ms), configured_ms)
}

fn command_with_compat_prelude(command: &str) -> String {
    format!("{BASH_COMPAT_PRELUDE}\n{SHELL_TIMEOUT_PRELUDE}\n{command}")
}

#[cfg(test)]
#[path = "bash_tests.rs"]
mod tests;
