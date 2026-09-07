# Write branch: keep partial work, gate dependents, bound the call — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:executing-plans, inline, one coordinator. Steps use checkbox syntax.

**Goal:** A write branch that runs out of time keeps its diff for the next attempt at the same task, its dependents are not dispatched against a baseline that lacks it, and the per-call time budget is an operator setting.

**Architecture:** Three small additions to the v2 worktree write path, no change to the authored-script dialect. (A) After a wave's branches complete and before cleanup, a branch that produced no manifest and is not accepted has its worktree diff captured to `write-coordination/stages/<call>/partial/<item>.patch`, recorded on its outcome, and its worktree retained. (B) When a wave prepares a branch whose canonical task has a recorded partial patch from an earlier branch in the run, the patch is applied to the fresh worktree and the agent is told so. (C) Before preparing a wave, each branch's `dependency_ids` are checked against accepted or no-op branch outcomes already recorded in the run; a branch with an unmet dependency is not dispatched and gets a typed `blocked_on_dependency` outcome the script's remediation loop can retry in canonical order. (D) `[workflow.generated] write_call_time_budget_secs` overrides the derived `3 × host_call_timeout_secs`.

**Tech Stack:** Rust 2024, existing `write_coordinator` git helpers (`run_git`), `WorkflowV2ResultStore::load_branch_outcomes`, `canonical_task_ids_from_generated_value`.

**Spec:** `docs/superpowers/tracked-defects.md` TD-057, TD-058 (root cause verified 2026-09-07 06:40: `call_time_budget = client.timeout_secs × 3` in `src/command/live_agent_dispatch.rs:56`; timed-out branch returns `manifest: None` and `cleanup_completed_worktree_wave` removes its worktree with `Succeeded`; no reader of `dependency_ids` in `crates/archon-workflow/src/v2/write/`).

## Global Constraints

- PRD-agnostic: no task ids, paths or domain words in engine code or fixtures beyond generic `TASK-001` style.
- Every changed Rust file under 500 lines. `worktree_branch_a.rs` is already 519 and `worktree_wave.rs` is 495: new logic goes in new child modules; those two files receive one-line hooks only.
- Red test before behaviour. Sabotage the production call site for each hook and watch the test fail.
- No cargo while an archon run is live. Commit before building; binary equals HEAD before any launch.
- Never stage `crates/archon-trading`; force-add the ledger.

## Files

- Create `crates/archon-workflow/src/v2/write/partial_work.rs` (capture, record, lookup, apply, preamble) and `partial_work_tests.rs`.
- Create `crates/archon-workflow/src/v2/write/dependency_gate.rs` (pure `unmet_dependencies`, blocked result) and `dependency_gate_tests.rs`.
- Modify `worktree_wave.rs`: hook A in `collect_worktree_wave_artifacts` and per-branch status in `cleanup_completed_worktree_wave`; hook B in `prepare_worktree_wave`; hook C in `run_one_worktree_wave`.
- Modify `worktree.rs`: `PreparedWorktreeBranch.resumed_partial: Option<PartialWork>`.
- Modify `worktree_branch_a.rs`: one line, `partial_work::with_resume_preamble(ctx.task, prepared.resumed_partial.as_ref())`.
- Modify `crates/archon-core/src/config/sections_workflow.rs`: `write_call_time_budget_secs: u32` default 0; `src/command/live_agent_dispatch.rs`: honour it.
- Modify `mod.rs` to register modules and tests. Ledger entries TD-057/TD-058 marked fixed with test names.

### Task 1: partial work capture and resume (A, B)

- [ ] Red: `partial_work_tests::captures_diff_of_a_timed_out_worktree_and_reapplies_it` — temp repo with one commit; worktree via `git worktree add`; modify a tracked file and add a new one; `capture_partial_work(ws, run_root, "call", "item")` returns `Some` with two files and a patch on disk; a second fresh worktree plus `apply_partial_work` shows both files; `with_resume_preamble` names both.
- [ ] Red: `partial_work_tests::latest_partial_is_found_by_canonical_task_id` — store two branch outcomes for `TASK-001`, only the later has `partial_work`; lookup returns it.
- [ ] Implement `partial_work.rs`; wire hook A and B; sabotage each hook (comment out the capture call; comment out the apply call) and confirm the wave-level test in `worktree_unapplied_tests.rs` style fails, restore.

### Task 2: dependency gate (C)

- [ ] Red: `dependency_gate_tests::branch_with_unmet_dependency_is_reported_and_met_one_is_not` — universe `TASK-001 → TASK-002`; outcomes: none, then `TASK-001` accepted; `unmet_dependencies` returns `{item-2: [TASK-001]}` then `{}`; a dependency outside the universe never blocks; `Noop` counts as met.
- [ ] Red: `dependency_gate_tests::blocked_result_is_typed_for_the_script` — `blocked_on_dependency_result` has status NeedsReview, gap id `blocked_on_dependency_<item>`, `data.blocked_on_dependency == [ids]`, `canonical_task_ids` carried.
- [ ] Implement; wire in `run_one_worktree_wave` before `prepare_worktree_wave`: filter blocked assignments out of the wave, save their outcomes with `save_write_branch_outcome`, push results. Sabotage the filter, confirm the gate test at wave level fails, restore.

### Task 3: budget override (D)

- [ ] Red: `live_agent_dispatch` test: override 900 gives a 900 s budget; 0 gives `timeout × 3`.
- [ ] Implement config field, validation range 0 or 300..=86_400, dispatch override.

### Task 4: verify, review, ship

- [ ] `cargo test -p archon-workflow --lib v2::write`, `cargo check --bins --tests`, root write tests.
- [ ] Hostile self-review of the diff: partial patch applied onto a moved baseline (wave commit landed in between) must not silently corrupt: `git apply --3way`, failure recorded on the prepared branch and told to the agent, never fatal.
- [ ] Ledger, commit by explicit path, clean-worktree build, deploy both binaries, set `write_call_time_budget_secs = 5400` in project-1. Relaunch only on Steven's word.
