//! `[tools]` and `[tools.cargo]`, plus the one place the Bash tool is built.
//!
//! Split out of `sections.rs` to keep that file under the 500-line ceiling.

use serde::{Deserialize, Serialize};

use super::PermissionsConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    /// Ceiling, in seconds, on how long a Bash command may run.
    pub bash_timeout: u64,
    /// Floor, in seconds, under which a caller-supplied `timeout` cannot drag a
    /// Bash command.
    ///
    /// The `timeout` argument on the Bash tool is model-supplied, and a model
    /// has no way to know how long a cold Rust build of this workspace takes. It
    /// used to be honoured downwards without limit, so a guessed two minutes
    /// killed builds that `bash_timeout` had budgeted far longer for. The floor
    /// makes the argument a request within a range rather than an unbounded
    /// veto.
    ///
    /// Clamped to `bash_timeout` when it exceeds it — the ceiling is the
    /// operator's word and always wins, so raising this can never extend a
    /// deliberately short `bash_timeout`.
    pub bash_timeout_floor: u64,
    pub bash_max_output: usize,
    pub max_concurrency: u8,
    /// Resource limits applied to `cargo` commands the agent runs.
    pub cargo: CargoResourceConfig,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            bash_timeout: 3600,
            bash_timeout_floor: 1800,
            bash_max_output: 102400,
            max_concurrency: 4,
            cargo: CargoResourceConfig::default(),
        }
    }
}

impl ToolsConfig {
    /// Build the `Bash` tool this config describes.
    ///
    /// Three call sites — the interactive session, the agent builder and the
    /// pipeline runner — each used to spell out the same struct literal, so
    /// every new field meant editing all three and any one of them could be
    /// missed. Permissions come in as an argument rather than being read from a
    /// global so this stays a pure function of its inputs.
    pub fn bash_tool(&self, permissions: &PermissionsConfig) -> archon_tools::bash::BashTool {
        archon_tools::bash::BashTool {
            timeout_secs: self.bash_timeout,
            timeout_floor_secs: self.bash_timeout_floor,
            max_output_bytes: self.bash_max_output,
            safe_commands: permissions.safe_commands.clone(),
            risky_commands: permissions.risky_commands.clone(),
            dangerous_commands: permissions.dangerous_commands.clone(),
            provider_env: None,
            cargo_limits: self.cargo.to_limits(),
            // Unrestricted as built. A subagent's registry is narrowed to its
            // tier afterwards, by `ToolRegistry::set_bash_isolation_tier`
            // (#184 M3); the main agent's is never narrowed.
            isolation_tier: archon_tools::isolation::IsolationTier::Shared,
        }
    }

    /// Build the `TerminalWrite` tool this config describes (#189 Phase 6).
    ///
    /// Alongside `bash_tool` and taking the same argument, because the two must
    /// classify a command identically: text typed into a persistent shell runs
    /// exactly as text passed to `Bash` does, and a tool that read a different
    /// list would be a way around the gate rather than a way to keep a shell
    /// open. Nothing from `[tools]` applies — the Bash timeout and output cap
    /// describe a call that waits for a command, which this one never does.
    pub fn terminal_write_tool(
        permissions: &PermissionsConfig,
    ) -> archon_tools::terminal_tools::TerminalWriteTool {
        archon_tools::terminal_tools::TerminalWriteTool {
            safe_commands: permissions.safe_commands.clone(),
            risky_commands: permissions.risky_commands.clone(),
            dangerous_commands: permissions.dangerous_commands.clone(),
        }
    }
}

/// Resource limits applied to agent-run `cargo` commands.
///
/// These were compile-time constants in `archon-tools`: every `cargo` command an
/// agent ran got `CARGO_BUILD_JOBS=1` with no way to change it. The intent was
/// sound — stop parallel agents thrashing one machine — but `1` is a single-core
/// build on any host, and combined with the Bash timeout it was the likeliest
/// cause of long builds being killed rather than finishing slowly.
///
/// Each field maps to one environment variable, and each is applied only as a
/// *default*: an explicit value already in the environment is left alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CargoResourceConfig {
    /// `CARGO_BUILD_JOBS`. `0` means derive from the host — see
    /// [`CargoResourceConfig::resolved_build_jobs`].
    pub build_jobs: u32,
    /// `CARGO_INCREMENTAL`. Off by default: agent builds are mostly cold, where
    /// incremental costs disk and time without paying it back.
    pub incremental: bool,
    /// `ARCHON_WORKFLOW_RESOURCE_CLASS`, the advisory label a command can read to
    /// tell how much of the machine it is entitled to.
    pub resource_class: String,
}

impl Default for CargoResourceConfig {
    fn default() -> Self {
        Self {
            build_jobs: 0,
            incremental: false,
            resource_class: "constrained".into(),
        }
    }
}

impl CargoResourceConfig {
    /// The `CARGO_BUILD_JOBS` value to apply, resolving `0` against the host.
    ///
    /// Auto is half the logical cores, minimum 1. Half rather than all because
    /// memory, not CPU, is what breaks these builds: this workspace links
    /// `aws-lc-sys`, `wasmtime` and `ort`, and several concurrent rustc
    /// processes on those peak at gigabytes each. Half the cores keeps a
    /// 16 GB-class laptop off swap while still being several times faster than
    /// the `1` this replaces. Hosts with memory to spare should set an explicit
    /// value; that is the entire point of the knob.
    pub fn resolved_build_jobs(&self) -> u32 {
        if self.build_jobs > 0 {
            return self.build_jobs;
        }
        let cores = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1);
        u32::try_from(cores / 2).unwrap_or(1).max(1)
    }

    /// Convert to the form the Bash tool holds.
    ///
    /// The conversion lives here rather than in `archon-tools` because
    /// `archon-tools` is the lower crate and cannot name this type — it is
    /// deliberately kept free of an `archon-core` dependency to break the cycle
    /// between the two. `build_jobs` is resolved on the way across, so the
    /// `0`-means-auto sentinel never leaves this crate.
    pub fn to_limits(&self) -> archon_tools::workflow_resource_env::CargoResourceLimits {
        archon_tools::workflow_resource_env::CargoResourceLimits {
            build_jobs: self.resolved_build_jobs(),
            incremental: self.incremental,
            resource_class: self.resource_class.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_build_jobs_resolves_to_at_least_one() {
        let cfg = CargoResourceConfig::default();
        assert_eq!(cfg.build_jobs, 0, "0 is the auto sentinel");
        assert!(cfg.resolved_build_jobs() >= 1, "cargo rejects a jobs of 0");
    }

    #[test]
    fn explicit_build_jobs_is_passed_through_unchanged() {
        let cfg = CargoResourceConfig {
            build_jobs: 11,
            ..Default::default()
        };
        assert_eq!(cfg.resolved_build_jobs(), 11);
    }

    /// The whole point of the constructor: config reaches the tool. A field
    /// added to `BashTool` and forgotten here shows up as one of these.
    #[test]
    fn bash_tool_carries_both_timeout_bounds_and_cargo_limits() {
        let tools = ToolsConfig {
            bash_timeout: 7200,
            bash_timeout_floor: 900,
            cargo: CargoResourceConfig {
                build_jobs: 6,
                incremental: true,
                resource_class: "full".into(),
            },
            ..Default::default()
        };

        let tool = tools.bash_tool(&PermissionsConfig::default());

        assert_eq!(tool.timeout_secs, 7200);
        assert_eq!(tool.timeout_floor_secs, 900);
        assert_eq!(tool.cargo_limits.build_jobs, 6);
        assert!(tool.cargo_limits.incremental);
        assert_eq!(tool.cargo_limits.resource_class, "full");
    }
}
