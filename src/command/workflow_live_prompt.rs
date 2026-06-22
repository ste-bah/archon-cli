use archon_workflow::StageRunRequest;
use serde_json::Value;

use super::workflow_live_runner::command_execution_stage;

pub(crate) fn workflow_prompt(request: &StageRunRequest) -> String {
    let input =
        serde_json::to_string_pretty(&request.input).unwrap_or_else(|_| request.input.to_string());
    let command_guidance = if command_execution_stage(request) {
        format!("\n{}", command_execution_guidance())
    } else {
        String::new()
    };
    let item_producer_guidance = item_producer_guidance(&request.input);
    format!(
        "## Workflow Task\n{}\n\n## Stage\nid: {}\nkind: {:?}\nprovider_tier: {:?}\nattempt: {}\ndepends_on: {:?}\n\n## Evidence Contract\nUse the `target_repository_root`, `task_evidence`, `source_files`, `dependencies`, and `fanout_item` fields in the stage input as primary evidence. For implementation stages, resolve relative target paths against `target_repository_root` and modify the repository files directly with the available tools. Do not ask whether to proceed, do not stop at a plan, and do not return a confirmation question; execute the stage now. If `task_evidence` is present, treat it as already-read task/source evidence; do not block because the original task file path is outside the isolated repository. A `source_files` entry with `exists:false` is valid greenfield evidence for a declared target file; do not block only because that target does not exist yet. If the implementation is already complete and no repository change is required, return exactly a JSON object containing `\"idempotent_noop\": true`, `\"status\": \"accepted\"`, and a concise `\"evidence\"` string, with no markdown around it. If required task files, source files, or upstream artifacts are absent, return a concise blocked report with `status: blocked`, the missing evidence, and do not invent findings.\n\n{RUNTIME_EVIDENCE_GUARDS}{item_producer_guidance}{command_guidance}\n\n## Stage Input\n```json\n{}\n```",
        request.task,
        request.stage_id,
        request.stage_kind,
        request.provider_tier,
        request.attempt,
        request.depends_on,
        input
    )
}

const RUNTIME_EVIDENCE_GUARDS: &str = concat!(
    "Runtime evidence guardrails:\n",
    "- For required-artifact inventory, repair, test, review, and report stages, trust dependency artifact fields named `checked`, `resolved`, `project_root`, `repository_root`, and `artifact_roots` when they are present. Relative `.archon/...` deliverables are project-root artifacts unless the inventory resolved path says otherwise; do not mark them missing merely because they are not under `target_repository_root`.\n",
    "- For post-remediation focused-test stages, if the remediation inventory is exactly `{\"items\": []}` and upstream review evidence has no current blockers, return `status: verified` with no-op evidence. Do not return `status: unverifiable` only because there were no remediation items.\n",
);

const ITEM_PRODUCER_GUIDANCE: &str = concat!(
    "\nStructured item output contract:\n",
    "- This stage is a machine-readable fanout item producer because a downstream stage iterates over this stage's `items`.\n",
    "- Return a single JSON object or YAML document with a top-level `items` array. Do not return only markdown/prose.\n",
    "- Do not return restored-context summaries, status-only prose, or a question asking what to do next; execute the stage and emit the structured item document now.\n",
    "- Each `items[]` entry must be an object with a stable `id` or `task_id`, a concise `task` or `summary`, concrete evidence, and `target_files` when repository changes may be required.\n",
    "- If there is genuinely no downstream implementation work, return `items: []` plus `completed_items` entries with `task_ids`/`work_unit_ids` or `canonical_task_ids`, `verified: true`, accepted status, and concrete evidence. Do not return only `idempotent_noop`.\n",
    "- If there is genuinely no downstream read-only work, return `{\"items\": []}` only when the evidence proves that no item should run; otherwise emit the concrete items needed by downstream fanout stages.\n",
);

fn item_producer_guidance(input: &Value) -> &'static str {
    if stage_extra_declares_items(input) {
        ITEM_PRODUCER_GUIDANCE
    } else {
        ""
    }
}

fn stage_extra_declares_items(input: &Value) -> bool {
    let Some(extra) = input.get("stage_extra") else {
        return false;
    };
    list_contains_ci(extra.get("outputs"), "items")
        || extra
            .get("produces")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("items"))
}

fn list_contains_ci(value: Option<&Value>, needle: &str) -> bool {
    value.and_then(Value::as_array).is_some_and(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .any(|value| value.eq_ignore_ascii_case(needle))
    })
}

const BASH_EXECUTION_GUIDANCE: &str = "For verification, focused-test, lint, build, or check stages, you MUST invoke Bash at least once before returning. If the stage input contains `verify_command`, run that exact command from `target_repository_root` instead of inventing a replacement. Do not set a Bash `timeout` field and do not wrap commands with shell-level `timeout`/`gtimeout` unless the workflow stage explicitly provides a timeout; rely on the configured `tools.bash_timeout`. Report exact commands, working directory, exit status, and pass/fail output. If Bash is unavailable or the command cannot be executed, return `status: failed` with the concrete execution failure. Do not return `status: blocked` merely because a command is expensive; run the focused command or report the concrete execution failure. Do not mark timed-out commands as completed or verified.";

fn command_execution_guidance() -> String {
    format!(
        "{BASH_EXECUTION_GUIDANCE}\n\n{FOCUSED_VERIFICATION_GUIDANCE}\n\n{}",
        cargo_command_policy_guidance()
    )
}

const FOCUSED_VERIFICATION_GUIDANCE: &str = concat!(
    "Focused verification selection:\n",
    "- For every language/build system, narrow at the test file, target, module, package, class, or test-id level before invoking the test runner.\n",
    "- Avoid broad project/package filters that still compile or run unrelated suites before filtering. Examples to avoid when a narrower target is knowable: `cargo test -p <large-package> <filter>`, whole-repo `pytest`, root `npm test`, root `gradle test`, or all-module Maven/Gradle runs.\n",
    "- Prefer exact commands such as Cargo `cargo test -p <pkg> --test <target> <filter>`, pytest `pytest path/to/test.py::test_name`, Gradle `./gradlew :module:test --tests ClassName.testName`, Maven `mvn -pl module -Dtest=ClassName#testName test`, npm/pnpm test-file or test-name filters, Go `go test ./pkg -run TestName`, or dotnet `dotnet test --filter Name~TestName`.\n",
    "- Use cheap source inspection (`rg`, `ls`, manifest/build files) to identify exact targets before running Bash. If the exact target cannot be determined, report the reason and choose the narrowest command available.\n",
);

fn cargo_command_policy_guidance() -> String {
    let profile = CargoHostProfile::detect();
    format!(
        "Cargo command policy for this host ({}):\n- Prefer exact package + test-target commands over workspace-wide commands.\n- Intermediate workflow test stages must not run `cargo check --workspace --tests` unless the stage or user explicitly requires it; reserve broad workspace checks for final quality gates.\n- {}\n- If upstream artifacts list stale Cargo commands that conflict with this host policy, adapt the commands and report the adaptation.",
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

pub(super) fn harness_planner_prompt(task: &str) -> String {
    format!(
        "Create an Archon dynamic workflow harness JavaScript script for this task:\n\n{task}\n\nRules:\n{HARNESS_PLANNER_RULES}\n\nReturn only JavaScript for workflow.js."
    )
}

const HARNESS_PLANNER_RULES: &str = concat!(
    "- Export exactly `export default async function workflow(w) { ... }`.\n",
    "- Return executable JavaScript only: no markdown fences, no prose, no comments, no TODO placeholders, and no restatement of these rules.\n",
    "- The function body must contain at least one awaited executable host call such as `await w.agent(\"discover\", { tier: \"planner\", task: \"...\" })`.\n",
    "- Every executable w.* host call must pass a non-empty stable string id as its first argument. Prefer literal ids. Inside bounded JavaScript loops, deterministic ids with a literal prefix are required, for example `\"implementation-wave-\" + waveIndex`; do not use opaque/random ids or repeat one static id across loop iterations.\n",
    "- Use only these host API calls: w.agent, w.fanout, w.parallel, w.reduce, w.tool, w.implementation, w.qualityGate, w.humanGate, w.checkpoint, w.saveArtifact, w.requireArtifact, w.finalReport.\n",
    "- Do not import modules, require modules, use eval/new Function, use filesystem/network APIs, shell APIs, providers, or model names.\n",
    "- Shape the workflow to the task with ordinary JavaScript control flow. Use variables for intermediate results, arrays for inventories, loops/branches for adaptive work, and host calls for all reading, editing, shell, review, test, artifact, and report work.\n",
    "- Host calls return typed result objects with `status`, `summary`, `items`, `outcomes`, and `result` fields; non-accepted semantic statuses do not throw. Inspect result.status/items/outcomes in JavaScript and decide whether to remediate, retry, reduce, ask the user, return, or continue.\n",
    "- Audit/review/research/planning is usually read-only: discover enough inventory, fan out reviews when useful, reduce findings, and qualityGate or finalReport the result.\n",
    "- Small known edits use w.implementation with targetFiles, optional verifyCommand, then review/qualityGate. Add remediation/fix calls when the task asks to fix review issues, remediate findings, or handle high-risk work.\n",
    "- Broad migrations use inventory variables -> implementation fanout with itemKind: \"implementation\" and targetFilesFromItem: true or explicit targetFiles -> focused verification/review -> adaptive remediation/fix loops as needed -> finalReport/qualityGate.\n",
    "- When review/remediation must continue until clean, use a bounded JavaScript `while` loop driven by typed result variables, for example `while (remaining.items.length > 0 && iteration <= 6) { ... }`. Do not unroll fixed remediation-pass-1/remediation-pass-2 stages unless the user explicitly asks for a fixed number of passes.\n",
    "- For decomposed PRD/task-directory objectives, do not create one host call per task ID or one remediation stage per task ID. Create compact typed inventory calls whose `items` feed fanout/parallel work; script variables and typed host-call results carry task coverage.\n",
    "- If the user says to fix every issue found before continuing, every adversarial review must feed a remediation inventory and fix stage before the next qualityGate or dependent workstream; use an adaptive bounded loop so the workflow decides from review output whether another remediation iteration is needed.\n",
    "- Every w.fanout or w.parallel that iterates work must pass the actual typed JavaScript array as its second argument, for example `w.fanout(\"id\", inventory.items, { ... })` or `w.parallel(\"id\", items, { ... })`. For static read-only discovery, define `const items = [...]` and pass `items` directly; `{ type: \"static_items\", items }` is accepted only as an explicit typed wrapper. Do not rely on prose from a prior call as the source.\n",
    "- Any upstream w.agent or w.reduce used as a fanout item source must have task text that requires a single parseable JSON/YAML object with top-level `items: [...]`; each implementation item must carry canonical task/work-unit ids, evidence, and target_files when downstream edits are expected.\n",
    "- Report-only deliverables must be w.agent or w.reduce artifacts plus w.requireArtifact before the quality gate; do not model reports as implementation work.\n",
    "- w.finalReport must include the evidence-producing implementation, review, verification, artifact, or reduce results as inputs; do not pass only a qualityGate result.\n",
    "- Every direct w.implementation call must include targetFiles: [\"relative/path\"].\n",
    "- Every implementation fanout must include itemKind: \"implementation\" and either targetFiles: [...] or targetFilesFromItem: true.\n",
    "- When using targetFilesFromItem: true, do not also add repo-root/project-root targetFiles as a fallback; the upstream items must carry concrete target_files ownership.\n",
    "- Static targetFiles on write-capable calls must name concrete files or narrow owned paths, never the repository root, project root, task directory, or another broad workspace root.\n",
    "- Use tier aliases only: planner, researcher, coder, critic, reducer, cheap, local, vision.\n",
    "- Keep maxParallelism <= 8 and total intended agent count <= 200.\n",
);

pub(super) fn harness_repair_prompt(task: &str, invalid_harness: &str, error: &str) -> String {
    format!(
        "The workflow harness failed safety validation or JavaScript execution setup.\n\nTask:\n{task}\n\nError:\n{error}\n\nRepair rules:\n- Return only executable JavaScript for workflow.js: no markdown fences, no prose, no comments, no TODO placeholders.\n- Export exactly `export default async function workflow(w) {{ ... }}`.\n- Include at least one awaited executable host call, for example `await w.agent(\"discover\", {{ tier: \"planner\", task: \"...\" }})`.\n- Every w.* host call must pass a non-empty stable string id as its first argument. Prefer literal ids. Inside bounded loops, deterministic ids with a literal prefix are required, for example `\"implementation-wave-\" + waveIndex`; never use random/opaque ids or reuse one static id across loop iterations.\n- Use only the allowed w.* host API; no imports, require, eval, filesystem, network, shell, providers, or model names inside the script.\n- Preserve provider neutrality.\n- Use JavaScript variables, branches, fanout/reduce, and bounded adaptive remediation loops to fit the task; do not generate one host call per task ID, a rigid per-task stage train, or fixed remediation-pass-1/remediation-pass-2 scaffolding unless explicitly requested.\n- Every w.fanout or w.parallel must pass a typed item source argument. For static read-only discovery use `const items = [...]` followed by `await w.parallel(\"id\", items, {{ ... }})`; an explicit `{{ type: \"static_items\", items }}` wrapper is also valid.\n- Add targetFiles for direct implementation work.\n- Use targetFilesFromItem: true only when an upstream inventory item will provide target_files.\n\nInvalid harness:\n```js\n{invalid_harness}\n```"
    )
}
