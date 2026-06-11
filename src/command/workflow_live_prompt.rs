use archon_workflow::StageRunRequest;

use super::workflow_live_runner::command_execution_stage;

pub(crate) fn workflow_prompt(request: &StageRunRequest) -> String {
    let input =
        serde_json::to_string_pretty(&request.input).unwrap_or_else(|_| request.input.to_string());
    let command_guidance = if command_execution_stage(request) {
        "\nFor verification, focused-test, lint, build, or check stages, use Bash to run the relevant command when it is available. Do not set a Bash `timeout` field and do not wrap commands with shell-level `timeout`/`gtimeout` unless the workflow stage explicitly provides a timeout; rely on the configured `tools.bash_timeout`. Report exact commands and pass/fail output. Do not return `status: blocked` merely because a command is expensive; run the focused command or report the concrete execution failure. Do not mark timed-out commands as completed or verified."
    } else {
        ""
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

pub(super) fn planner_prompt(task: &str) -> String {
    format!(
        "Create an archon.workflow.v1 YAML plan for this task:\n\n{task}\n\nRules:\n- Use schema: archon.workflow.v1.\n- Use stage kinds: agent, fanout, reduce, tool, checkpoint, quality_gate, human_gate, implementation.\n- Use provider_tier aliases only: planner, researcher, coder, critic, cheap, vision, local, reducer.\n- Do not set stage.provider or stage.model.\n- If the task names a repository/root directory to modify, set top-level `target_repository_root` to that exact Git/Cargo repository path. Do not set it to the Archon project directory unless that directory is the actual source repository.\n- Omit the top-level provider_tiers map entirely. If you must include it, map only real tier names (planner, researcher, coder, critic, cheap, vision, local, reducer) to the literal value auto, and never to a provider or model name.\n- You may set stage.task for the concise objective of that stage.\n- Include at least discovery, fanout/review, reduce/synthesis, and quality gate stages.\n- Fan-out contract (MANDATORY): a `kind: fanout` stage that iterates over upstream items MUST set `foreach: ${{<producer-stage-id>.items}}` where `<producer-stage-id>` is one of its `depends_on` stages. Do NOT use a decorative `fanout: {{over: ...}}` block to express iteration; `over`/`respect_dependencies` are never executed and will be rejected.\n- The producer stage referenced by `foreach` MUST be an upstream stage that emits a structured items document and declares `outputs: [items]`. Its agent task MUST instruct it to return a JSON/YAML object of the exact form `{{\"items\": [ {{...}}, {{...}} ]}}` (one object per work unit, e.g. per task or per wave). Without a real items producer the fan-out cannot run.\n- If the requested workflow must modify repository files, do NOT use a text-only agent/fanout as the final implementation. Use `kind: implementation` with `expected_target_files` for known files, or use `kind: fanout` plus `item_kind: implementation` when iterating task items. Each implementation fanout item MUST include a non-empty `target_files` array.\n- Never set `item_kind` on `agent`, `reduce`, `tool`, `checkpoint`, `quality_gate`, `human_gate`, or `implementation` stages. `item_kind` is valid only on `kind: fanout`, and only as `item_kind: implementation`.\n- A review fan-out is read-only by default: use `kind: fanout` without `item_kind` unless that exact stage is expected to edit files.\n- Implementation stages and implementation fanout items are write-capable and must be followed by focused tests, adversarial review, a remediation inventory, remediation implementation fanout, post-remediation focused tests, post-remediation adversarial review, final synthesis, and final quality gate.\n- The remediation inventory stage MUST depend on the first adversarial review, declare `outputs: [items]`, and emit exactly `{{\"items\": []}}` when there are no blockers. Each non-empty remediation item MUST include finding_id, related_task_id, target_files, failure, required_fix, and required_tests.\n- The remediation implementation fanout MUST set `foreach: ${{<remediation-inventory-stage>.items}}`, `item_kind: implementation`, and `allow_empty_items: true` in stage extra or input so a clean review can no-op instead of failing.\n- The final quality gate MUST depend on the post-remediation synthesis/report, not directly on the initial adversarial review; otherwise stale pre-fix failures will poison a successfully remediated run.\n- Set `verify_command` when a focused verification command is knowable.\n- Keep max_parallelism <= 8 and max_agents <= 200.\n- Add learning_hooks for sona, reasoning_bank, and world_model.\n- Return YAML only."
    )
}

pub(super) fn repair_prompt(task: &str, invalid_yaml: &str, error: &str) -> String {
    format!(
        "The workflow YAML failed validation.\n\nTask:\n{task}\n\nError:\n{error}\n\nInvalid YAML:\n```yaml\n{invalid_yaml}\n```\n\nReturn repaired archon.workflow.v1 YAML only."
    )
}
