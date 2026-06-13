use archon_workflow::StageRunRequest;

use super::workflow_live_runner::command_execution_stage;

pub(crate) fn workflow_prompt(request: &StageRunRequest) -> String {
    let input =
        serde_json::to_string_pretty(&request.input).unwrap_or_else(|_| request.input.to_string());
    let command_guidance = if command_execution_stage(request) {
        format!("\n{}", command_execution_guidance())
    } else {
        String::new()
    };
    format!(
        "## Workflow Task\n{}\n\n## Stage\nid: {}\nkind: {:?}\nprovider_tier: {:?}\nattempt: {}\ndepends_on: {:?}\n\n## Evidence Contract\nUse the `target_repository_root`, `source_files`, `dependencies`, and `fanout_item` fields in the stage input as primary evidence. For implementation stages, resolve relative target paths against `target_repository_root` and modify the repository files directly with the available tools. A `source_files` entry with `exists:false` is valid greenfield evidence for a declared target file; do not block only because that target does not exist yet. If required task files, source files, or upstream artifacts are absent, return a concise blocked report with `status: blocked`, the missing evidence, and do not invent findings.{command_guidance}\n\n## Stage Input\n```json\n{}\n```",
        request.task,
        request.stage_id,
        request.stage_kind,
        request.provider_tier,
        request.attempt,
        request.depends_on,
        input
    )
}

const BASH_EXECUTION_GUIDANCE: &str = "For verification, focused-test, lint, build, or check stages, you MUST invoke Bash at least once before returning. Do not set a Bash `timeout` field and do not wrap commands with shell-level `timeout`/`gtimeout` unless the workflow stage explicitly provides a timeout; rely on the configured `tools.bash_timeout`. Report exact commands, working directory, exit status, and pass/fail output. If Bash is unavailable or the command cannot be executed, return `status: failed` with the concrete execution failure. Do not return `status: blocked` merely because a command is expensive; run the focused command or report the concrete execution failure. Do not mark timed-out commands as completed or verified.";

fn command_execution_guidance() -> String {
    format!(
        "{BASH_EXECUTION_GUIDANCE}\n\n{}",
        cargo_command_policy_guidance()
    )
}

fn cargo_command_policy_guidance() -> String {
    let profile = CargoHostProfile::detect();
    format!(
        "Cargo command policy for this host ({}):\n- Prefer focused package/test filters over workspace-wide commands.\n- Intermediate workflow test stages must not run `cargo check --workspace --tests` unless the stage or user explicitly requires it; reserve broad workspace checks for final quality gates.\n- {}\n- If upstream artifacts list stale Cargo commands that conflict with this host policy, adapt the commands and report the adaptation.",
        profile.label(),
        profile.jobs_guidance()
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CargoHostProfile {
    Macos,
    Wsl2,
    NativeLinux,
    Windows,
    Other,
}

impl CargoHostProfile {
    fn detect() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            if linux_is_wsl2() {
                Self::Wsl2
            } else {
                Self::NativeLinux
            }
        } else {
            Self::Other
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Macos => "macOS",
            Self::Wsl2 => "WSL2",
            Self::NativeLinux => "native Linux",
            Self::Windows => "native Windows",
            Self::Other => "unknown platform",
        }
    }

    fn jobs_guidance(self) -> &'static str {
        match self {
            Self::Macos => {
                "Native macOS: do not add `-j1` or `--jobs 1` merely because repository docs mention WSL2. Omit Cargo job flags by default, or use an explicit configured job cap when the stage/user asks for one."
            }
            Self::NativeLinux => {
                "Native Linux: do not add `-j1` or `--jobs 1` merely because repository docs mention WSL2. Omit Cargo job flags by default, or use an explicit configured job cap when the stage/user asks for one."
            }
            Self::Windows => {
                "Native Windows: do not add `-j1` or `--jobs 1` merely because repository docs mention WSL2. Omit Cargo job flags by default, or use an explicit configured job cap when the stage/user asks for one."
            }
            Self::Wsl2 => {
                "WSL2 or low-memory host: use `-j1`/`--jobs 1` and keep test threads bounded, for example `-- --test-threads=2`, unless the stage/user explicitly supplies a different safe cap."
            }
            Self::Other => {
                "Unknown host: prefer focused commands and do not hard-code `-j1`/`--jobs 1` unless the stage/user explicitly identifies the current host as WSL2 or low-memory."
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_is_wsl2() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|version| {
            let version = version.to_ascii_lowercase();
            version.contains("microsoft") || version.contains("wsl")
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn linux_is_wsl2() -> bool {
    false
}

pub(super) fn planner_prompt(task: &str) -> String {
    format!(
        "Create an archon.workflow.v1 YAML plan for this task:\n\n{task}\n\nRules:\n{PLANNER_RULES}"
    )
}

const PLANNER_RULES: &str = concat!(
    "- Use schema: archon.workflow.v1.\n",
    "- Use stage kinds: agent, fanout, reduce, tool, checkpoint, quality_gate, human_gate, implementation.\n",
    "- Use provider_tier aliases only: planner, researcher, coder, critic, cheap, vision, local, reducer.\n",
    "- Do not set stage.provider or stage.model.\n",
    "- If the task names a repository/root directory to modify, set top-level `target_repository_root` to that exact Git/Cargo repository path. Do not set it to the Archon project directory unless that directory is the actual source repository.\n",
    "- Omit the top-level provider_tiers map entirely. If you must include it, map only real tier names (planner, researcher, coder, critic, cheap, vision, local, reducer) to the literal value auto, and never to a provider or model name.\n",
    "- You may set stage.task for the concise objective of that stage.\n",
    "- Include at least discovery, fanout/review, reduce/synthesis, and quality gate stages.\n",
    "- Fan-out contract (MANDATORY): a `kind: fanout` stage that iterates over upstream items MUST set `foreach: ${{<producer-stage-id>.items}}` where `<producer-stage-id>` is one of its `depends_on` stages. Do NOT use a decorative `fanout: {{over: ...}}` block to express iteration; `over`/`respect_dependencies` are never executed and will be rejected.\n",
    "- The producer stage referenced by `foreach` MUST be an upstream stage that emits a structured items document and declares `outputs: [items]`. Its agent task MUST instruct it to return a JSON/YAML object of the exact form `{{\"items\": [ {{...}}, {{...}} ]}}` (one object per work unit, e.g. per task or per wave). Without a real items producer the fan-out cannot run.\n",
    "- If the requested workflow must modify repository files, do NOT use a text-only agent/fanout as the final implementation. Use `kind: implementation` with `expected_target_files` for known files, or use `kind: fanout` plus `item_kind: implementation` when iterating task items. Each implementation fanout item MUST include a non-empty `target_files` array.\n",
    "- Do not model report-only, readiness-only, audit-only, acceptance-only, or external/project-artifact deliverables as repository implementation fanouts. If a task's deliverable is a report, readiness artifact, acceptance report, coverage matrix, adversarial review, or file under the Archon project outside the target repository, create an `agent` or `reduce` stage that produces that artifact/report and make later gates depend on it.\n",
    "- If a decomposed task mixes repository edits and required reports/artifacts, split it into separate stages: implementation fanout for repository edits, then an artifact/report stage for the required non-code deliverables. Never let an empty implementation target inventory skip a required report/readiness/artifact deliverable.\n",
    "- Never set `item_kind` on `agent`, `reduce`, `tool`, `checkpoint`, `quality_gate`, `human_gate`, or `implementation` stages. `item_kind` is valid only on `kind: fanout`, and only as `item_kind: implementation`.\n",
    "- A review fan-out is read-only by default: use `kind: fanout` without `item_kind` unless that exact stage is expected to edit files.\n",
    "- Implementation stages and implementation fanout items are write-capable and must be followed by focused tests, adversarial review, a remediation inventory, remediation implementation fanout, post-remediation focused tests, post-remediation adversarial review, final synthesis, and final quality gate.\n",
    "- The remediation inventory stage MUST depend on the first adversarial review, declare `outputs: [items]`, and emit exactly `{{\"items\": []}}` when there are no blockers. Each non-empty remediation item MUST include finding_id, related_task_id, target_files, failure, required_fix, and required_tests.\n",
    "- The remediation implementation fanout MUST set `foreach: ${{<remediation-inventory-stage>.items}}`, `item_kind: implementation`, and `allow_empty_items: true` in stage extra or input so a clean review can no-op instead of failing.\n",
    "- The final quality gate MUST depend on the post-remediation synthesis/report, not directly on the initial adversarial review; otherwise stale pre-fix failures will poison a successfully remediated run.\n",
    "- Cargo verification commands MUST be platform-aware: use WSL2/low-memory `-j1` only when the current host or stage explicitly requires it. Native macOS, native Linux, and native Windows should omit `-j1`/`--jobs 1` by default or use a configured job cap.\n",
    "- Focused test stages MUST prefer package/test filters. Do not place `cargo check --workspace --tests` in intermediate wave test stages unless the user explicitly requested that broad gate; reserve broad workspace checks for final quality gates.\n",
    "- Set `verify_command` when a focused verification command is knowable.\n",
    "- Keep max_parallelism <= 8 and max_agents <= 200.\n",
    "- Add learning_hooks for sona, reasoning_bank, and world_model.\n",
    "- Return YAML only.",
);

pub(super) fn repair_prompt(task: &str, invalid_yaml: &str, error: &str) -> String {
    format!(
        "The workflow YAML failed validation.\n\nTask:\n{task}\n\nError:\n{error}\n\nInvalid YAML:\n```yaml\n{invalid_yaml}\n```\n\nReturn repaired archon.workflow.v1 YAML only."
    )
}
