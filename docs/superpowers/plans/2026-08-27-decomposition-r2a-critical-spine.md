# Decomposition R2a Critical Spine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the R2a engine-native decomposition spine, prove it through a synthetic full lifecycle and a trading decomposition-only run, and stop for evidence review before any R2b hardening or trading implementation.

**Architecture:** A fixed `FixedDecompositionV1` script runs in the existing persisted v3 engine. The raw `w.hostCommand` bridge accepts only symbolic capabilities plus bounded stdin; the binary resolves a digested host-owned catalog, supervises trusted non-detaching Archon children, and alone publishes audited staged output with exact-byte receipts. Existing freeze/lint/trace kernels remain authoritative, while a centralized finalizer runs an immutable observe-only commandless-floor observer after terminal state and event persistence.

**Tech Stack:** Rust, Tokio, QuickJS/rquickjs, serde/serde_json, BLAKE3, clap, existing Archon workflow/result-store/event/TUI/provider abstractions, direct Cargo test harnesses.

**Spec:** `docs/superpowers/specs/2026-08-27-decomposition-r2-engine-native-design.md` at commit `84b1d8466d7cbd40afdf9905ed8fe39c060fc623`

## Global Constraints

- Work on `fix/verification-criteria-coverage`, starting from committed spec revision `84b1d8466d7cbd40afdf9905ed8fe39c060fc623`.
- R2a only: do not implement the isolated acceptance world, `AdoptedPredecessorReceipt`, executable snapshots/handshakes, cross-run leases/CAS, retained-directory-handle/no-follow publication, crash-perfect journals, crash-active author ledgers, OS-locked cross-process state/event transactions, progress outboxes, observer claims, or startup observer recovery.
- Target fresh/empty task roots and one active executor. Refuse a second active decomposition for the same task root; do not claim concurrent-run correctness.
- Models never author executables, argv, cwd, environment, catalogs, locks, pins, receipts, destination paths, write sets, reuse keys, or script structure.
- Raw model-authored command, nested verifier, and residual fail-closed text never executes on the host. R2a records post-terminal observer-operational deferral.
- Existing `freeze-acceptance`, `freeze-skeleton`, task-file lint, task-set lint, and requirements-trace kernels remain the only validators. `GateEnvelopeV1` serializes their typed output; it does not revalidate it.
- `FixedDecompositionV1` uses the persisted v3 engine only. Do not add a bespoke Rust phase engine or agent/Bash relay.
- `w.hostCommand` must be on the authoritative raw `__archonW`; a prelude-only helper is dead code for scripts using zero helpers.
- `HostCommand` must traverse ordinary persisted call records/checkpoints. Do not route it through the `runTool` early-return path.
- Provider routing for fixed author and freeze-judge calls is `ConfiguredOnly`; ignore ambient `ANTHROPIC_BASE_URL` and reject unapproved repository endpoint overrides before any model request.
- Author tools are exactly required `Read`, `Grep`, `Glob`, `CartographerScan`, plus optional `LeannSearch` only when registered. Never pass Bash, Write, Edit, lock writers, or provenance writers. An empty allowlist is forbidden.
- Acceptance and skeleton authoring allow six logical attempts; body authoring allows ten. Each logical attempt has a 1,500-second backstop. Pause/cancel does not advance the logical attempt; timeout, truncation, malformed output, and rejection do.
- Policy findings do not block in observe. Operational/integrity failures always stop. `AcceptedWithShadowFindings` exists only in decomposition run/phase/body metadata.
- Run-end observer authority is hard-coded `ObserveOnly`, even when global `gate_mode=enforce`. `FixedDecompositionV1` is never observer-eligible.
- Legacy whole-chain absence remains byte-silent and freeze-unaware. Preserve the existing mortal-admission test and its call-site sabotage at `execute_generated_v2_run`.
- Never edit, stage, commit, delete, move, or reformat any path under `crates/archon-trading`. Protect the current 15 modified and 2 untracked files by path-and-byte snapshot before every verification/build/proof gate.
- Never use `git add -A`, `git add -u`, `git commit -a`, push, or trigger CI. Stage only explicit reviewed R2a paths.
- Never run Cargo while any Archon workflow/decomposition/proof/TUI workflow is active. Never run two Cargo operations concurrently.
- Never terminate the cognitive daemon, LiteLLM `:1234`, OpenBB `:6900`, tradingview-mcp, unrelated Cargo jobs, or macOS security services.
- No PRD/domain-specific identifiers in production code, test names, comments, fixed script, or committed generic proof harness. External proof values arrive only through runtime inputs.
- No file may cross the repository's 500-line growth guard. Extract a focused module before adding behavior to a near-limit file.
- One reviewed local R2a implementation commit precedes the single release build. Build after commit, deploy both binaries atomically, run synthetic proof before trading proof, and stop for evidence review before R2b/R3/R4 or any trading implementation.

---

## Single-Coordinator Verification Protocol

Exactly one coordinator owns `cargo`, `rustc`, `rustdoc`, `clippy`, Rust test executables, repository scripts that invoke Cargo, release builds, live proof runs, and process termination. Subagents may author tests and implementation but never execute them.

Every subagent prompt must include this text verbatim:

> Do not invoke `cargo`, `rustc`, `rustdoc`, `clippy`, any Rust test executable, or any repository script that may invoke them. Do not alter `CARGO_TARGET_DIR`, start an Archon workflow or decomposition run, deploy a binary, or issue process-kill commands. Return proposed test names and commands to the coordinator for queued execution. Do not edit, stage, or otherwise disturb any path under `crates/archon-trading`.

### Coordinator baseline

Before Task 1, record in a coordinator-owned ledger under `/private/tmp/archon-r2a-<run-id>/`:

- repository root, branch, starting revision, Rust toolchain, coordinator PID, and run ID;
- SHA-256 and status for every modified/untracked protected trading file;
- allowed write paths for the active task;
- one fresh local-system `CARGO_TARGET_DIR`, for example `/private/tmp/archon-r2a-target-<run-id>`;
- every compile command, cwd, environment, PID/PGID, descendants, timestamps, exit status, harness path/hash, direct-test result, and watchdog observations.

Use `CARGO_INCREMENTAL=0` and `CARGO_BUILD_JOBS=2` for focused compilation. Use one build job only for a measured resource issue or the release build, and record the reason.

### Mandatory preflight before every Cargo/fmt/clippy/build invocation

Run from `/Volumes/Externalwork/archon-cli/archon-cli`:

```bash
workflow_count="$(ps -Ao comm | grep -c '^\./archon$' || true)"
test "$workflow_count" = "0"
ps -Ao pid=,ppid=,pgid=,etime=,state=,%cpu=,comm=,args=
```

Then:

1. Recompute protected trading hashes and compare them with baseline.
2. Compare all other working-tree changes with the active task's explicit write allowlist.
3. Identify every Cargo, rustc, rustdoc, clippy-driver, cc/clang/linker, Rust test harness, repository-cwd process, and process holding the selected target directory or Cargo lock.
4. Record PID, PPID, PGID, elapsed time, state, CPU, full args, cwd, ancestry, target directory, and ledger owner.
5. Require every previous coordinator operation to be terminal and reaped.
6. Stop on active workflow/decomposition/proof/TUI work, target ownership, unknown process ownership, protected mutation, or unrelated changed paths. Never kill an unidentified process.

### Compile receipt and direct harness

Compile one target per invocation and capture Cargo's JSON stream without replacing its exit status:

```bash
env CARGO_TARGET_DIR="$TARGET" CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test --manifest-path /Volumes/Externalwork/archon-cli/archon-cli/Cargo.toml \
  -p "$PACKAGE" $TARGET_SELECTOR --no-run --message-format=json-render-diagnostics
```

The coordinator parses `compiler-artifact`, requires exactly one matching executable beneath `$TARGET`, canonicalizes it, records SHA-256, and then invokes that exact unchanged harness:

```bash
"$HARNESS" "$FULLY_QUALIFIED_TEST" --exact --nocapture --test-threads=1
```

Use `--test-threads=1` for process, signal, global-state, publication, run-root, lifecycle, status, event, and proof-sensitive tests. Use at most two threads only for demonstrably independent pure tests. A source change invalidates the compile receipt.

Watchdog ceilings are separate: 20 minutes for warm focused compile, 45 minutes for fresh/root compile, five minutes for a direct pure unit test, 15 minutes for process/publication/resume integration, 30 minutes for a broad package harness, 60 minutes for release build, and 1,500 seconds per model author call. Every 20 seconds record process state, elapsed time, CPU/memory, descendants, log size/mtime/tail, and phase. Silence triggers diagnosis of process tree, CPU, output changes, target lock, open files, endpoint-security inspection, orphaned descendants, and reap state; silence alone is not a deadlock.

On timeout, capture diagnostics, verify the PGID belongs to the coordinator and is not its own group, send TERM, wait a bounded grace period, send KILL only to surviving coordinator-owned descendants, then wait/reap and prove none survived.

### TDD and review handoff

For every behavior slice:

1. The owning subagent writes the focused test and returns its exact filter; it runs nothing.
2. The coordinator preflights, compiles, and runs the exact harness to record RED for the intended missing behavior.
3. The owning subagent writes the minimum production change.
4. The coordinator recompiles, runs GREEN, then adjacent regression filters from the same receipt.
5. An independent reviewer inspects the diff and evidence without Cargo.
6. Critical/Important findings are fixed and reverified before transferring file ownership.

A helper-only test is insufficient for wiring. Each load-bearing call site gets a discriminating sabotage: remove/bypass the call, require the test to fail, restore exactly, recompile, and require GREEN.

---

## Planned File Ownership

### `archon-workflow` contracts and raw bridge

- Create `crates/archon-workflow/src/v2/host_command.rs` — typed request/result/catalog contract and canonical length-framed call identity.
- Create `crates/archon-workflow/src/v2/gate_envelope.rs` — closed finding/operational envelope.
- Create `crates/archon-workflow/src/v2/publication.rs` — prepared manifests and exact-byte receipts.
- Create `crates/archon-workflow/src/v2/decomposition.rs` — run kind, phases, attempts, subject dispositions, fixed-run identity/state.
- Create `crates/archon-workflow/src/v2/finalization.rs` — launch snapshot and finalization record.
- Create `crates/archon-workflow/src/v2/script/source.rs` — authoritative raw `w` source assembly extracted from the near-limit helper.
- Create `crates/archon-workflow/src/v2/script/host_command.rs` — shared request parser and dry-run recorder.
- Modify `crates/archon-workflow/src/v2/host_api.rs`, `v2/mod.rs`, `v2/script/mod.rs`, `v2/script/helpers_a.rs`, `v2/script/dry_run_a.rs`, and `v2/script/dry_run_b.rs` only for registration/delegation.
- Create `crates/archon-workflow/tests/host_command_contract.rs` and `decomposition_state.rs`.

### Binary host runtime

- Create `src/command/workflow_host_command_catalog.rs` — immutable fixed catalog, token rebinding, digest, postconditions.
- Create `src/command/workflow_host_command_supervisor.rs` — direct-child process mechanics and bounded drains/reaping.
- Create `src/command/workflow_host_command_exec.rs` — persisted call orchestration.
- Create `src/command/workflow_host_command_publish.rs` — staging, mutation audits, parent publication, receipts.
- Create `src/command/workflow_host_command_tests.rs` and `workflow_host_command_publication_tests.rs`.
- Modify `src/command/workflow_live_v2_script_host_exec.rs` and `workflow_live_v2_script_host_state.rs` only as dispatch/persistence adapters.
- Extract HostCommand dispatch from near-limit `workflow_live_v2_host_dispatch.rs` into `workflow_live_v2_host_command_dispatch.rs`.

### Authoring and authoritative gates

- Create `src/command/workflow_live_v2_client_raw.rs` — trusted raw-outcome dispatch that bypasses structured-result normalization/repair.
- Create `src/command/workflow_provider_route.rs` — configured-only route resolution and provenance.
- Create `src/command/workflow_gate_envelope.rs` — exhaustive mapping from existing finding constructors.
- Modify `workflow_live_v2_client.rs`, `workflow_live_v3_author.rs`, `workflow_live_provider_env.rs`, `workflow_gate.rs`, `workflow_freeze_cli.rs`, `workflow_task_set.rs`, `workflow_task_set_publish.rs`, `workflow_task_set_judge.rs`, `topology_lint.rs`, `topology_lint/task_file.rs`, `topology_lint/task_set.rs`, `requirement_trace/evaluation.rs`, and `requirement_trace/verdict.rs` by delegation; do not duplicate validators.

### Fixed run, lifecycle, UI, and observer

- Create `src/command/workflow_decompose_v1.js` — immutable generic Phase 0/A/B/C/D/E script embedded with `include_str!`.
- Create `src/command/workflow_decompose.rs` — launcher/composition and fixed-run identity.
- Create `src/command/workflow_decompose_progress.rs` — event→log→transient ordering and coalescing.
- Create `src/command/workflow_decompose_tests.rs`, `workflow_decomposition_phase_tests.rs`, and `workflow_decomposition_resume_tests.rs`.
- Modify `crates/archon-workflow/src/v2/result_store_records.rs`, `result_store.rs`, `restart.rs`, `events.rs`, `tui_events.rs`, plus `src/cli_args/strategy_actions_workflow.rs`, `src/command/workflow.rs`, `workflow_live_v2_run.rs`, `workflow_status_detail.rs`, and `tui_workflow_ui_sink.rs`.
- Create `crates/archon-workflow/src/v2/declarative_floor.rs` and `crates/archon-workflow/tests/declarative_floor_observer.rs`.
- Create `src/command/workflow_live_v2_finalizer.rs`, `workflow_run_end_observer.rs`, and focused tests; remove authoritative terminal emission from the existing script-host branches by delegation.

### Proof harnesses

- Create `tests/workflow_decomposition_synthetic_live.rs` and generic fixtures under `tests/fixtures/decomposition-synthetic/`.
- Create `tests/workflow_decomposition_external_prd_live.rs`; accept external PRD/task/protected/evidence paths through runtime environment and commit no domain-specific value.

File ownership is exclusive per task. Where a later task touches an earlier file, the coordinator records an explicit ownership handoff after the earlier review gate.

---

### Task 1: Add shared R2a contracts and the authoritative raw `w.hostCommand`

**Files:**
- Create: `crates/archon-workflow/src/v2/host_command.rs`
- Create: `crates/archon-workflow/src/v2/gate_envelope.rs`
- Create: `crates/archon-workflow/src/v2/publication.rs`
- Create: `crates/archon-workflow/src/v2/decomposition.rs`
- Create: `crates/archon-workflow/src/v2/finalization.rs`
- Create: `crates/archon-workflow/src/v2/script/source.rs`
- Create: `crates/archon-workflow/src/v2/script/host_command.rs`
- Create: `crates/archon-workflow/tests/host_command_contract.rs`
- Create: `crates/archon-workflow/tests/decomposition_state.rs`
- Modify: `crates/archon-workflow/src/v2/host_api.rs`
- Modify: `crates/archon-workflow/src/v2/mod.rs`
- Modify: `crates/archon-workflow/src/v2/script/mod.rs`
- Modify: `crates/archon-workflow/src/v2/script/helpers_a.rs`
- Modify: `crates/archon-workflow/src/v2/script/dry_run_a.rs`
- Modify: `crates/archon-workflow/src/v2/script/dry_run_b.rs`

**Interfaces:**

```rust
pub struct HostCommandRequest {
    pub command_id: String,
    pub stdin: Option<String>,
}

pub struct HostCommandResult {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub timed_out: bool,
    pub interrupted: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub gate_envelope: Option<GateEnvelopeV1>,
    pub publication_receipt: Option<PublicationReceiptV1>,
}

pub fn host_command_call_id(
    command_id: &str,
    catalog_digest: &str,
    starting_binary_revision: &str,
    resolved_tokens: &BTreeMap<String, String>,
    stdin: &[u8],
) -> String;

pub enum WorkflowRunKind {
    AuthoredTaskWorkflow,
    LegacyDecomposed,
    FixedDecompositionV1,
    FixedOrSavedScript,
}
```

`WorkflowV2HostMethod::HostCommand` serializes/parses as `hostCommand`. `WorkflowV2HostOptions` gains a typed `host_command: Option<HostCommandRequest>`; process authority is never accepted through `extra`.

- [ ] **Step 1: Write contract and bridge tests first.** Add exact tests `host_command_method_parse_and_as_str_round_trip`, `host_command_serializes_as_camel_case_host_command`, `host_command_rejects_empty_command_id`, `host_command_accepts_only_bounded_stdin`, `host_command_rejects_script_authored_process_fields`, `host_command_call_identity_is_domain_separated_and_length_framed`, `raw_w_host_command_reaches_authoritative_host_bridge`, `host_command_is_available_without_v3_convenience_prelude`, `dry_run_records_host_command_without_spawn_or_write`, and `dry_run_host_command_stub_contains_complete_result_shape`. The identity test must use two ambiguous concatenation pairs and assert different digests.
- [ ] **Step 2: Record RED through the coordinator.** Compile `cargo test -p archon-workflow --lib --no-run` and `cargo test -p archon-workflow --test host_command_contract --no-run`; run exact filters and require missing enum/member/type failures rather than syntax or fixture errors.
- [ ] **Step 3: Define the serializable types.** Add `HostCommandRequest`, result, catalog capability fields, prepared/publication receipt records, gate envelope, closed remediation scopes, run kind, phase/attempt/disposition, launch snapshot, and finalization record. Use serde defaults so old metadata omitting run kind/observer snapshot decodes to its legacy semantics without changing old serialized bytes.
- [ ] **Step 4: Implement canonical identity.** Encode the domain, command ID, catalog digest, binary revision, sorted token map keys/values, and exact stdin as length-prefixed byte slices before BLAKE3. Reuse the repository's canonical stable-hash/publication-identity machinery where compatible; do not copy the spec's visual raw concatenation literally.
- [ ] **Step 5: Extract source assembly before adding behavior.** Move `script_source` assembly from the 476-line `helpers_a.rs` into `script/source.rs`, leaving a delegation/re-export. Move HostCommand dry-run parsing out of the 485-line `dry_run_a.rs`. Run existing normalized-source and dry-run filters from the already compiled harness to prove behavior-preserving extraction.
- [ ] **Step 6: Add raw `hostCommand` directly to frozen `__archonW`.** Validate non-empty capability ID and string/null stdin, use a sequential transport correlation ID, and pass only `{commandId, stdin}` to `__archonCall`. The host replaces the correlation ID with the canonical call identity before persistence.
- [ ] **Step 7: Make dry-run/live parsing one grammar.** Both use `parse_host_command_request`; dry-run validates the declaration, emits a normal `WorkflowV2CallExecution`, performs no spawn/write, returns every result field with `dryRun: true`, and reports remediation topology incomplete.
- [ ] **Step 8: Verify GREEN and adjacent regressions.** Recompile both targets, run every new exact test plus existing script normalization, fire-and-forget, `runTool`, and dry-run tests.
- [ ] **Step 9: Prove call-site sensitivity.** Temporarily remove only the `hostCommand` member from `__archonW`; require `raw_w_host_command_reaches_authoritative_host_bridge` to fail while prelude tests remain irrelevant. Restore exactly, recompile, and require GREEN. Ensure no sabotage marker remains.
- [ ] **Step 10: Independent contract review.** Reviewer checks serde backward compatibility, no process fields in request/extra, canonical hashing, raw bridge placement, dry-run parity, and the 500-line guard. Transfer ownership only after Critical/Important findings are closed.

**Task verification targets:** `archon-workflow --lib`, `host_command_contract`, `decomposition_state`.

---

### Task 2: Serialize authoritative gate findings and add staged command entry points

**Files:**
- Create: `src/command/workflow_gate_envelope.rs`
- Create: `src/command/workflow_gate_envelope_tests.rs`
- Modify: `src/command/workflow_gate.rs`
- Modify: `src/command/workflow_freeze_cli.rs`
- Modify: `src/command/workflow_task_set.rs`
- Modify: `src/command/workflow_task_set_publish.rs`
- Modify: `src/command/workflow_task_set_judge.rs`
- Modify: `src/command/topology_lint.rs`
- Modify: `src/command/topology_lint/task_file.rs`
- Modify: `src/command/topology_lint/task_set.rs`
- Modify: `src/command/requirement_trace/evaluation.rs`
- Modify: `src/command/requirement_trace/verdict.rs`
- Modify: focused freeze/lint/trace test modules only

**Interfaces:**

```rust
pub fn classify_gate_finding(
    gate_id: GateId,
    finding: &GateFinding,
) -> WorkflowResult<GatePolicyFinding>;

pub fn build_gate_envelope(
    gate_id: GateId,
    report: serde_json::Value,
    findings: &[GateFinding],
    operational_error: Option<GateOperationalError>,
) -> WorkflowResult<GateEnvelopeV1>;

pub enum TaskFileLintScope {
    PhaseLocal,
    CompleteSet,
}
```

Existing `GateFinding` is extended with an explicit remediation scope/source kind at construction. No classifier derives scope from `text` or `subject` prose.

- [ ] **Step 1: Add exhaustive typed-envelope RED tests.** Cover every existing R2 finding constructor across acceptance freeze, skeleton freeze, task-file lint, task-set lint, and requirements trace. Assert exact text preservation and one of `CandidateArtifact`, `Skeleton`, `PrdInput`, `Body`, `InheritedPredecessor`, or `Operational`. Unknown/missing constructors and unknown operational kinds must error.
- [ ] **Step 2: Add routing discrimination tests.** Add `phase_c_task_file_gate_excludes_set_level_coverage`, `phase_d_routes_skeleton_findings_without_body_loop`, `phase_d_prd_input_failure_stops_without_retry`, and `phase_d_external_body_finding_is_operational`. Use fixtures where prose could misleadingly name another subject, proving routing reads the enum only.
- [ ] **Step 3: Record RED in the root binary harness.** Compile `cargo test -p archon-cli-workspace --bin archon --no-run`; run exact envelope/routing filters and require failures caused by absent typed fields and current task-file parent coverage.
- [ ] **Step 4: Extend existing finding constructors.** Add explicit scope/source kind where findings originate. Classify malformed/duplicate/zero PRD obligations as `PrdInput`; unreadable/corrupt inputs and writes as `Operational`; inherited observe-stamped predecessor findings as `InheritedPredecessor`.
- [ ] **Step 5: Add a phase-local task-file evaluator.** Reuse parsing, frozen-tuple, body, and predecessor kernels but skip parent-directory population/coverage/trace until Phase D. Preserve ordinary CLI `--task-file` behavior by selecting `CompleteSet` there.
- [ ] **Step 6: Add host-owned envelope output.** Gate commands write `GateEnvelopeV1` to an exact declared side-channel path passed by the trusted command mode. Human stdout/stderr remains unchanged. Missing/malformed required envelope is operational.
- [ ] **Step 7: Add stdin/staged-output modes to freeze and body landing.** Refactor existing freeze preparation/publication and skeleton reconciliation rather than duplicating them. Child mode consumes bounded candidate stdin, validates/judges through existing kernels, writes only a complete prepared manifest/tree, and never renames live outputs or creates a committed receipt.
- [ ] **Step 8: Preserve stop-reason-before-parse.** Freeze judge calls reject max-token/length/missing/malformed provider outcomes before JSON parsing or any staged/live write.
- [ ] **Step 9: Verify GREEN and authoritative-kernel parity.** Run all new envelope/routing filters plus existing `workflow_gate`, freeze, topology lint, requirement trace, `task_set_contract`, `task_skeleton`, and `task_set_edges` targets.
- [ ] **Step 10: Prove wiring sensitivity.** Temporarily remove the Phase C scope argument and require the partial-population test to fail; temporarily replace enum routing with body-default routing and require Phase D tests to fail. Restore, recompile, and require GREEN.
- [ ] **Step 11: Independent gate-policy review.** Reviewer accounts for every existing constructor, confirms no duplicate validation, confirms Phase C exclusion and Phase D no-body-loop behavior, and checks legacy CLI output remains compatible.

**Task verification targets:** root `--bin archon`, `task_set_contract`, `task_skeleton`, `task_set_edges`, existing lint/trace/freeze suites.

---

### Task 3: Build the immutable capability catalog and trusted process supervisor

**Files:**
- Create: `src/command/workflow_host_command_catalog.rs`
- Create: `src/command/workflow_host_command_supervisor.rs`
- Create: `src/command/workflow_host_command_exec.rs`
- Create: `src/command/workflow_host_command_tests.rs`
- Modify: `src/command/mod.rs` or its existing path registrations

**Interfaces:**

```rust
pub fn fixed_decomposition_catalog(
    starting_binary_revision: &str,
) -> WorkflowResult<CommandCapabilityCatalog>;

pub fn resolve_host_command(
    request: &HostCommandRequest,
    catalog: &CommandCapabilityCatalog,
    context: &HostCommandResolutionContext,
) -> WorkflowResult<ResolvedHostCommand>;

#[async_trait]
pub trait HostCommandProcessAdapter: Send + Sync {
    async fn execute(
        &self,
        request: ResolvedHostCommandRequest,
        control: HostCommandControl,
    ) -> WorkflowResult<ObservedProcessOutcome>;
}

pub async fn supervise_process_group(
    command: SupervisedCommand,
    control: &dyn WorkflowRunControl,
) -> WorkflowResult<SupervisedProcessOutput>;
```

Catalog IDs are exactly `freeze-acceptance`, `freeze-skeleton`, `land-task-body`, `task-set-lint`, and `requirements-trace`. Each binds program identity, canonical argv template, token sources/validators, cwd, stdin delivery, named environment profile, limits, write set, envelope policy, and postcondition.

- [ ] **Step 1: Write request-authority RED tests.** Add `host_command_request_binds_stdin_only` and rejection tests for executable, argv, cwd, environment, timeout/limits, write set, destination, and reuse key. Include shell metacharacters in stdin and prove they remain bytes, never argv/program text.
- [ ] **Step 2: Write catalog/token RED pairs.** Cover unknown capability, every authority-bearing field changing catalog digest, canonical path/traversal/symlink rejection, `FrozenTaskId` grammar, direct-child filename, and host re-read skeleton tuple rebinding. Test stale/mutated frozen tuples are rejected before spawn.
- [ ] **Step 3: Write supervisor RED tests with fake child fixtures.** Add separate stdout/stderr, simultaneous large drains, stdout/stderr overflow, timeout, pause, cancel, parent-liveness closure, spawn failure, process-group cleanup, known descendants, reap failure, and detached-capability construction refusal. The fake records argv/env/cwd/stdin without invoking a provider.
- [ ] **Step 4: Record RED.** Compile root `--bin archon`; run exact tests with one thread and prove missing resolver/supervisor behavior.
- [ ] **Step 5: Construct the catalog from host constants and existing subcommands.** Set approved limits: acceptance/skeleton stdin+streams 2 MiB and 1,500s; body stdin+streams 1 MiB and 300s; set gates no stdin, streams 4 MiB and 300s. Persist names/resolution sources, never environment values.
- [ ] **Step 6: Implement token rebinding and postconditions.** Canonicalize from trusted parents, reject symlink descent/escape, re-read serde-validated frozen skeleton, and bind complete set-gate manifest inputs. Recheck expected old digests and frozen tuple immediately before spawn/commit.
- [ ] **Step 7: Implement environment profiles.** Start children with `env_clear()`. Freeze gets only exact configured provider names; lint/trace/body gets no environment. Never inherit HOME/PATH/key/endpoint unless explicitly declared. Unrelated sentinels must be absent.
- [ ] **Step 8: Implement direct process supervision.** Spawn no shell, establish a dedicated process group and liveness pipe, concurrently drain bounded streams, race completion/timeout/control/parent closure, TERM then KILL known coordinator-owned descendants, await reaping and pipe closure, and mark every abnormal outcome non-reusable. Dropping a future is not cancellation.
- [ ] **Step 9: Verify GREEN and leak checks.** Run every authority/token/supervisor test serially; after each process test inspect descendants and require none remain.
- [ ] **Step 10: Prove no-shell and reaping call-site sensitivity.** Replace direct spawn with a test-only shell path and require metacharacter tests to fail; bypass the reap wait and require leak/reap tests to fail. Restore exactly and rerun GREEN.
- [ ] **Step 11: Independent command-security review.** Reviewer verifies every capability's exact program/argv/env/write set, no script override, bounds, process-tree assumptions, and honest trusted-child—not sandbox—claim.

**Task verification targets:** root `--bin archon`; existing `v2_agent_adapter` only for adjacent process/client compatibility.

---

### Task 4: Add parent-only staged publication and persisted HostCommand dispatch

**Files:**
- Create: `src/command/workflow_host_command_publish.rs`
- Create: `src/command/workflow_host_command_publication_tests.rs`
- Create: `src/command/workflow_live_v2_host_command_dispatch.rs`
- Modify: `src/command/workflow_live_v2_script_host_exec.rs`
- Modify: `src/command/workflow_live_v2_script_host_state.rs`
- Modify: `src/command/workflow_live_v2_host_dispatch.rs`
- Modify: `crates/archon-workflow/src/v2/result_store_records.rs`
- Modify: `crates/archon-workflow/src/v2/result_store.rs`

**Interfaces:**

```rust
pub fn prepare_staging(
    run_root: &Path,
    call_id: &str,
    capability: &CommandCapability,
) -> WorkflowResult<CommandStaging>;

pub fn audit_prepared_publication(
    staging: &CommandStaging,
    prepared: &PreparedPublicationV1,
    capability: &CommandCapability,
    sentinels: &LiveMutationSentinels,
) -> WorkflowResult<AuditedPublication>;

pub fn publish_audited(
    audited: AuditedPublication,
) -> WorkflowResult<PublicationReceiptV1>;

pub fn host_command_record_is_reusable(
    record: &WorkflowV2CallRecord,
    expected_call_id: &str,
    receipt: &PublicationReceiptV1,
    postcondition: &CommandPostcondition,
) -> WorkflowResult<bool>;
```

- [ ] **Step 1: Write publication RED tests.** Add `prepared_freeze_nonzero_exit_publishes_nothing`, `prepared_freeze_stdout_overflow_publishes_nothing`, `prepared_freeze_timeout_publishes_nothing`, `prepared_body_cancel_restores_prior_body`, `parent_commit_requires_exact_declared_staged_tree`, `unexpected_staged_file_is_operational`, `mutation_sentinel_change_refuses_publication`, `prior_digest_change_before_commit_refuses_publication`, `receipt_records_exact_final_bytes`, and symlink/path-escape pairs.
- [ ] **Step 2: Write persistence/reuse RED tests.** Add `host_command_uses_persisted_call_record_path`, `host_command_does_not_route_through_run_tool`, `host_command_updates_checkpoint_after_committed_receipt`, `host_command_nonzero_exit_returns_process_data`, `host_command_operational_failure_is_not_reusable`, `host_command_reuse_requires_exact_publication_receipt`, `later_shadow_jsonl_append_does_not_invalidate_receipt_membership`, and `different_internally_valid_freeze_is_not_reusable`.
- [ ] **Step 3: Record RED.** Compile root harness plus `v2_result_contracts`; run serial filters and require absent publication/dispatch behavior.
- [ ] **Step 4: Implement run-owned staging and manifest audit.** Register exact target/prior/temp/backup/final digests before staging; enumerate the full staged tree; reject missing, extra, duplicate, symlink, escape, type, size, or digest mismatch. Child paths never appear in durable receipt.
- [ ] **Step 5: Implement parent commit using existing atomic publisher machinery.** Generalize existing sibling staging/backup/rename and `GatePublicationPermit` behavior. Parent commits only after zero exit, bounded closed streams, no interruption/truncation/infrastructure failure, child reaping, valid envelope, complete manifest, prior-digest check, and mutation sentinel check.
- [ ] **Step 6: Compute the receipt from live bytes after commit.** Include command/invocation identity, exact immutable file digests, stable shadow record IDs/canonical record digests/membership proof, prior digests, parent-established process metadata, and commit sequence. Never hash the mutable whole shadow JSONL.
- [ ] **Step 7: Route HostCommand through ordinary persisted execution.** Parse typed request, resolve host identity, create/update normal call state/checkpoint, execute, publish, persist accepted result+receipt, then expose the result to QuickJS. Keep `runTool`'s early return unchanged and separate.
- [ ] **Step 8: Implement R2a interruption semantics honestly.** Prepared-only output is discarded. Deterministically complete backup restores; ambiguous incomplete commit stops operationally with cleanup/restart remedy. Never adopt or mark partial output accepted.
- [ ] **Step 9: Verify GREEN and adjacent result-store suites.** Run all new tests plus `v2_result_contracts`, `v2_resume`, and existing freeze atomic rollback tests.
- [ ] **Step 10: Prove dispatch sensitivity.** Temporarily route HostCommand beside `RUN_TOOL_METHOD`; require `host_command_uses_persisted_call_record_path` and checkpoint tests to fail. Temporarily publish before final process observation; require all prepared-failure tests to fail. Restore and rerun GREEN.
- [ ] **Step 11: Independent persistence/publication review.** Reviewer traces prepare→observe→audit→commit→receipt→record ordering and checks that reuse requires exact produced bytes, terminal subject outcome, and authoritative postcondition.

**Task verification targets:** root `--bin archon`, `v2_result_contracts`, `v2_resume`, `workflow_live_status_write_coordination`.

---

### Task 5: Add trusted raw provider outcomes, fixed tool policy, and configured-only routing

**Files:**
- Create: `src/command/workflow_live_v2_client_raw.rs`
- Create: `src/command/workflow_provider_route.rs`
- Create: `src/command/workflow_decomposition_author_tests.rs`
- Modify: `src/command/workflow_live_v2_client.rs`
- Modify: `src/command/workflow_live_v3_author.rs`
- Modify: `src/command/workflow_live_provider_env.rs`
- Modify: `crates/archon-workflow/src/v2/agent_adapter_a.rs` only for the result-mode request contract, not raw parsing
- Modify: `crates/archon-workflow/src/llm_retry.rs` by reusable typed retry/backstop composition

**Interfaces:**

```rust
pub enum AgentResultMode {
    Structured,
    RawOutcome,
}

pub struct RawAgentOutcome {
    pub content: String,
    pub stop_reason: String,
}

pub async fn run_agent_raw(
    client: &dyn WorkflowLlmClient,
    request: WorkflowV2AgentRequest,
    route: &TrustedProviderRouteSnapshot,
    backstop: Duration,
) -> WorkflowResult<RawAgentOutcome>;

pub enum ProviderEndpointPolicy {
    AmbientAllowed,
    ConfiguredOnly,
}

pub fn fixed_decomposition_tool_policy(
    registry: &ToolRegistry,
) -> WorkflowResult<Vec<String>>;
```

- [ ] **Step 1: Write RED raw-outcome tests.** Add `trusted_raw_outcome_bypasses_structured_parse_and_repair`, `one_author_attempt_makes_exactly_one_provider_request`, `max_tokens_raw_outcome_is_returned_before_candidate_parse`, `malformed_raw_content_never_reaches_workflow_result_repair`, and `raw_mode_is_rejected_for_untrusted_authored_scripts`.
- [ ] **Step 2: Write tool/route RED tests.** Add exact allowlist, missing required tool, empty fallback refusal, optional Leann registration, Bash/Write/Edit absence, hostile ambient endpoint ignored, unapproved repository endpoint rejected before request, trusted route provenance persisted without secrets, and unrelated environment sentinel absent.
- [ ] **Step 3: Record RED.** Compile root harness, `archon-core --lib`, and `v2_agent_adapter`; run exact filters and prove current normal adapter attempts `parse_agent_output`/repair.
- [ ] **Step 4: Implement a distinct trusted fixed-run raw adapter.** `resultMode: rawOutcome` is admitted only for embedded `FixedDecompositionV1`; it returns provider `content` and typed `stop_reason` directly. It never invokes `WorkflowV2AgentAdapter::parse_agent_output`, normalization, or repair.
- [ ] **Step 5: Build the non-empty read-only policy from the actual registry.** Require exact production names after filtering; optionally add `LeannSearch` only when present and read-only. Refuse launch before first dispatch if any required tool is absent or list is empty.
- [ ] **Step 6: Implement `ConfiguredOnly`.** Resolve only explicit CLI/operator or user-level provider configuration outside the repository. Ignore ambient endpoint variables; reject repository endpoint/profile/key/proxy overrides unless exact operator approval is durably snapshotted. Persist non-secret route/profile origin and endpoint digest.
- [ ] **Step 7: Compose typed transient retries inside one logical-attempt backstop.** Generalize existing `llm_retry` classifier/loop. The 1,500-second wall backstop includes retries. Pause/cancel returns typed interruption without advancing; expiry/truncation/malformed/rejection advances in the phase state machine.
- [ ] **Step 8: Verify GREEN and current structured-adapter regressions.** Run new tests plus all existing V2 adapter/client/provider-env and retry tests. Normal authored workflows must still normalize/repair structured results.
- [ ] **Step 9: Prove raw call-site sensitivity.** Temporarily send fixed raw requests through normal `parse_agent_output`; require opaque candidate and max-token tests to fail. Restore and rerun GREEN.
- [ ] **Step 10: Independent provider review.** Reviewer verifies route trust boundaries, exact tools, no empty-default semantics, one request per logical attempt, typed stop reason before parse, and no secrets in persistence/logs.

**Task verification targets:** root `--bin archon`, `archon-core --lib`, `v2_agent_adapter`.

---

### Task 6: Implement the immutable fixed decomposition script and phase router

**Files:**
- Create: `src/command/workflow_decompose_v1.js`
- Create: `src/command/workflow_decompose.rs`
- Create: `src/command/workflow_decomposition_phase_tests.rs`
- Create: `src/command/workflow_decomposition_test_support.rs`
- Modify: `src/command/workflow_live_v2.rs` only to register/delegate fixed-run composition
- Modify: `src/command/workflow_live_v2_run.rs` only for explicit run-kind dispatch

**Interfaces:**

```rust
pub struct FixedDecompositionContext {
    pub project_root: PathBuf,
    pub prd_path: PathBuf,
    pub task_root: PathBuf,
    pub gate_mode: GateMode,
    pub provider_route: TrustedProviderRouteSnapshot,
    pub ui_sink: SharedWorkflowUiSink,
}

pub async fn run_fixed_decomposition(
    context: FixedDecompositionContext,
) -> WorkflowResult<WorkflowTerminalSummary>;
```

The script owns only orchestration. It calls `w.agent` in raw mode and symbolic `w.hostCommand`; it treats candidate/envelope content as opaque except for typed host result fields.

- [ ] **Step 1: Build deterministic fake-client/process fixtures.** `ScriptedDecompositionClient` queues `Complete`, `Truncated`, `Malformed`, `TransportFailure`, `WaitForControl`, and `BackstopExpired`, and records phase, subject, attempt, route, tools, sanitized prompt digest, stop reason, and whether a provider request occurred. Host fakes return typed envelopes/receipts only.
- [ ] **Step 2: Write full phase-order RED tests.** Add `fake_client_executes_phase_zero_a_b_all_c_d_e_in_order`, `empty_task_root_reaches_first_acceptance_author_call`, `bodies_do_not_start_before_acceptance_and_skeleton_receipts`, `phase_e_reconciles_every_receipt_digest_and_subject_outcome`, and `receipt_for_rejected_candidate_cannot_complete_subject`.
- [ ] **Step 3: Add phase/error RED tests.** Cover malformed/duplicate/zero PRD immediate stop; acceptance/skeleton six attempts; body ten attempts; pause/cancel no advance; timeout/truncation/malformed/rejection advance; exact finding returned to responsible author; inherited predecessor loud/non-retrying/nonblocking; candidate frozen mismatch retry; accepted artifact mutation operational; Phase D typed routing/no body loop; observe exhaustion shadow acceptance; operational failure never shadow-accepts.
- [ ] **Step 4: Add exemplar drift tests.** Serialize real Rust acceptance/skeleton/body exemplars into prompts, prove authoritative validators accept them, and prove discriminating invalid counterparts fail. Do not hand-maintain JSON examples that can drift.
- [ ] **Step 5: Record RED.** Compile root harness and `archon-workflow --lib`; run exact filters, requiring missing fixed script/dispatch failures.
- [ ] **Step 6: Embed one immutable generic script with `include_str!`.** Implement Phase 0 identity; A acceptance; B skeleton; C body fanout using host-read tuples; D canonical-manifest set gates; E full-chain/receipt/outcome/postcondition reconciliation. No domain names, model-generated control flow, Bash, or parallel validator.
- [ ] **Step 7: Add explicit run-kind dispatch.** Replace overloaded `script_lifecycle` interpretation with backward-compatible run kind. Existing authored, legacy-decomposed, and saved/fixed scripts retain their path; `FixedDecompositionV1` alone loads the embedded source/catalog/args.
- [ ] **Step 8: Implement phase-local retry and disposition.** Retry only candidate-local typed scopes. Preserve inherited predecessor event links. In observe, policy exhaustion retains the last structurally valid landed candidate and records run-metadata-only `AcceptedWithShadowFindings`. Operational/integrity failures stop.
- [ ] **Step 9: Implement Phase E using existing integrity kernels.** Call `validate_full_chain`, `compare_task_set`, exact receipt byte checks, terminal subject outcomes, and postconditions; do not create a second reconciliation validator.
- [ ] **Step 10: Verify GREEN and phase regressions.** Run all phase tests plus `v2_decomposed_prd`, `lifecycle_audit`, authored-script lifecycle, and legacy execution filters.
- [ ] **Step 11: Prove phase wiring sensitivity.** Remove A receipt gating before B and require ordering test failure; move C before skeleton receipt and require failure; skip `compare_task_set` in E and require reconciliation test failure. Restore and rerun GREEN.
- [ ] **Step 12: Independent phase/policy review.** Reviewer maps every spec phase and error taxonomy row to code/tests, confirms typed routing/no prose inference, and confirms no R2b feature or second engine entered.

**Task verification targets:** root `--bin archon`, `archon-workflow --lib`, `v2_decomposed_prd`, `lifecycle_audit`.

---

### Task 7: Persist fixed-run identity, phase state, receipts, and resume admission

**Files:**
- Create: `src/command/workflow_decomposition_resume_tests.rs`
- Modify: `crates/archon-workflow/src/v2/result_store_records.rs`
- Modify: `crates/archon-workflow/src/v2/result_store.rs`
- Modify: `crates/archon-workflow/src/v2/restart.rs`
- Modify: `src/command/workflow_live_v2_run.rs`
- Modify: `src/command/workflow_decompose.rs`

**Interfaces:**

```rust
pub struct FixedRunIdentityV1 {
    pub template_version: String,
    pub starting_binary_revision: String,
    pub script_digest: String,
    pub catalog_digest: String,
    pub project_root_identity: String,
    pub prd_identity: String,
    pub task_root_identity: String,
}

pub fn verify_fixed_resume_identity(
    persisted: &FixedRunIdentityV1,
    current: &FixedRunIdentityV1,
) -> WorkflowResult<()>;

pub fn subject_is_resume_reusable(
    state: &FixedDecompositionStateV1,
    subject: &DecompositionSubject,
    call: &WorkflowV2CallRecord,
    receipt: &PublicationReceiptV1,
    current_postcondition: &CommandPostconditionEvaluation,
) -> WorkflowResult<bool>;
```

- [ ] **Step 1: Write run/resume RED tests.** Add `fixed_decomposition_run_is_persisted_before_executor_spawn`, `fixed_decomposition_resume_requires_binary_script_and_catalog_identity`, project/PRD/task-root mismatch pairs, `fixed_decomposition_resume_skips_only_with_receipt_outcome_chain_and_postcondition`, `preexisting_chain_without_run_receipt_is_not_silently_skipped`, `second_active_decomposition_for_same_task_root_is_refused`, `legacy_generated_v2_run_remains_freeze_unaware`, and `fixed_decomposition_never_creates_observer_intent`.
- [ ] **Step 2: Add mutation/requeue RED tests.** Acceptance/skeleton exact receipt skips; partial/corrupt publication stops; body receipt+terminal+lint skip; missing/mutated body requeues; stale set manifest resumes D; paused author retries same attempt; incomplete publication requires cleanup.
- [ ] **Step 3: Record RED.** Compile root harness, `v2_resume`, `v2_result_contracts`, and `store_events`; run serial exact filters.
- [ ] **Step 4: Persist run identity before executor spawn.** Store run kind/template, binary/script/catalog, canonical project/PRD/task identities, arguments, fixed source, command catalog, log path, phase/body state, attempts, and ordinary call/checkpoint state. A barrier test must observe all records before executor progress.
- [ ] **Step 5: Implement resume identity refusal.** Compare current invoking binary revision, embedded script digest, catalog digest, and canonical inputs before any provider/host call. Return a named no-deploy/restart remedy on mismatch.
- [ ] **Step 6: Implement four-way skip admission.** Require accepted call identity, committed exact-byte receipt, terminal subject disposition, and current authoritative postcondition. File existence or internal freeze validity alone never skips.
- [ ] **Step 7: Implement one-active-executor launcher refusal.** Reuse/adapt the existing workflow/run ownership root lock where possible. Refuse a second active run/task root; do not implement R2b lease/CAS/stale-owner recovery.
- [ ] **Step 8: Normalize orderly pause/cancel and stale Running.** Resume re-enters the same logical attempt after orderly control interruption. Process crash may normalize stale Running to the last durable phase but cannot adopt incomplete publication.
- [ ] **Step 9: Verify GREEN and legacy regressions.** Run all resume tests plus existing generated V2 resume/restart/result-store suites.
- [ ] **Step 10: Re-run mortal legacy admission sabotage.** Run `unfrozen_legacy_task_sets_reach_v3_author_in_observe_and_enforce` GREEN; temporarily block the first line of `execute_generated_v2_run`, require both cases to fail before fake author, restore exactly, touch/recompile as needed, and require GREEN. Require no sabotage marker and no legacy fixture rewrite.
- [ ] **Step 11: Independent resume review.** Reviewer checks every skip predicate, run identity, second-run refusal, no silent predecessor adoption, incomplete-publication stop, and legacy isolation.

**Task verification targets:** root `--bin archon`, `v2_resume`, `v2_result_contracts`, `store_events`, exact mortal admission filter.

---

### Task 8: Add durable progress, CLI/TUI lifecycle, and status detail

**Files:**
- Create: `src/command/workflow_decompose_progress.rs`
- Create: `src/command/workflow_decompose_tests.rs`
- Modify: `crates/archon-workflow/src/events.rs`
- Modify: `crates/archon-workflow/src/tui_events.rs`
- Modify: `src/cli_args/strategy_actions_workflow.rs`
- Modify: `src/command/workflow.rs`
- Modify: `src/command/workflow_status_detail.rs`
- Modify: `src/command/tui_workflow_ui_sink.rs`
- Modify: `crates/archon-core/src/skills/workflow_prd_spec.rs`
- Modify: `src/command/prd_pipeline_layout_tests.rs`
- Modify: `src/cli_args/workflow_task_set_parse_tests.rs`

**Interfaces:**

```rust
pub struct DecompositionProgressReporter {
    pub run_id: String,
    pub log_path: PathBuf,
    pub next_event_id: u64,
    pub ui_sink: SharedWorkflowUiSink,
}

impl DecompositionProgressReporter {
    pub fn append_durable(
        &mut self,
        event: &DecompositionEvent,
    ) -> WorkflowResult<()>;

    pub fn try_emit_transient(&self, event: &DecompositionEvent);
}
```

CLI action is exactly:

```text
workflow decompose --prd <PATH> --tasks <DIR> [--yes]
```

No `continue` alias is added.

- [ ] **Step 1: Write parser/wiring RED tests.** Add `workflow_decompose_parses_prd_tasks_and_yes`, required-argument pairs, `workflow_decompose_does_not_add_continue_alias`, `decompose_gate_mode_off_refuses_before_run_creation`, `slash_workflow_decompose_persists_run_before_spawn`, and `cli_workflow_decompose_requires_yes_for_live_execution`.
- [ ] **Step 2: Write progress/UI RED tests.** Add durable event→flushed log→transient order, stable IDs across orderly resume, 64-KiB event refusal, log sanitization, resume markers, write/flush failure, bounded UI saturation/coalescing, closed UI non-stall, CLI progress drain, status immediate after launch, retained executor handle, TUI shutdown cancellation, and child reap.
- [ ] **Step 3: Record RED.** Compile root harness, `archon-core --lib`, `archon-tui --lib`, and `store_events`; run exact filters serially.
- [ ] **Step 4: Add first-class command surfaces.** CLI requires `--yes`; slash persists and returns run ID before background execution. `gate_mode=off` returns the exact spec remedy before run creation/path publication/provider construction.
- [ ] **Step 5: Replace skill relay structurally.** `/workflow-prd-spec` delegates to the host decompose entry; it never asks an agent to run Bash or emits a bare `archon` fallback. Add a call-site test, not a prose snapshot alone.
- [ ] **Step 6: Add typed durable events.** Persist phase, attempt, model-in-flight, host command, findings, subject disposition, phase completion, decomposition completion, and observer events without prompts, candidate bodies, secrets, or environment values.
- [ ] **Step 7: Implement event→log→transient order.** Append the normal event, append and fsync/flush `.decompose.log`, then nonblocking enqueue. Coalesce transient progress on saturation and emit a durable-log marker. Failure of durable event/log persistence is operational.
- [ ] **Step 8: Implement TUI ownership and shutdown.** Retain join handle/cancellation token; orderly closure signals cancellation, waits for active trusted host cleanup/reaping, and never accepts the run. CLI long-running mode prints progress rather than discarding it.
- [ ] **Step 9: Extend status detail.** Render run kind/template, binary/script/catalog identities, phase/subject, logical attempt/backstop, active provider/capability, call/body totals, shadows, last error, log path, resume skips, and finalization/observer state.
- [ ] **Step 10: Verify GREEN and lifecycle regressions.** Run new tests plus command parsing, workflow prompt contract, TUI, event store, pause/cancel/status, and generated lifecycle suites.
- [ ] **Step 11: Prove handoff/order sensitivity.** Remove host delegation from the skill and require the structural test to fail. Swap log/transient order and require ordering test to fail. Drop executor handle and require cleanup test to fail. Restore and rerun GREEN.
- [ ] **Step 12: Independent UI/lifecycle review.** Reviewer checks no agent relay, persistence-before-spawn, nonblocking transient delivery, log content boundaries, TUI cleanup, status completeness, and no R2b outbox/transaction claims.

**Task verification targets:** root `--bin archon`, `archon-core --lib`, `archon-tui --lib`, `store_events`, `workflow_live_prompt_contract`.

---

### Task 9: Extract one pure declarative-floor kernel

**Files:**
- Create: `crates/archon-workflow/src/v2/declarative_floor.rs`
- Create: `crates/archon-workflow/tests/declarative_floor_observer.rs`
- Modify: `crates/archon-workflow/src/v2/deliverable_contract.rs`
- Modify: `crates/archon-workflow/src/v2/mod.rs`
- Modify: the existing deliverable-verifier generation path only to call the shared evaluator

**Interfaces:**

```rust
pub fn collect_declarative_floor_input(
    artifact_root: &Path,
    contract: &WorkflowV2DeliverableContract,
) -> WorkflowResult<DeclarativeFloorInput>;

pub fn evaluate_declarative_floor(
    contract: &WorkflowV2DeliverableContract,
    input: &DeclarativeFloorInput,
) -> DeclarativeFloorEvaluation;
```

The evaluator judges only artifact/record data. It cannot render or execute a command.

- [ ] **Step 1: Write parity RED tests.** Add passing/failing floor fixtures and assert the new kernel's result exactly matches the existing verifier's declarative predicates. Add `existing_verifier_calls_shared_declarative_floor_kernel` as a wiring tripwire.
- [ ] **Step 2: Write no-execution RED tests.** Use `PanicOnExecute` to prove `AcceptanceCheck::Command`, floor `typed_verifier_command`, and residual `fail_closed_check` are classified/deferred before any HostCommand, Bash registry, or shell adapter. Add residual structure, all-pass+stale-gap, duplicate/unknown/extra-gap tests using existing `validate_residual_gaps`.
- [ ] **Step 3: Record RED.** Compile `archon-workflow --lib` and `declarative_floor_observer`; run exact filters.
- [ ] **Step 4: Separate collection from judgment.** Collect only declared artifact/record facts, then implement a deterministic pure evaluation result. Keep filesystem reads out of `evaluate_declarative_floor`.
- [ ] **Step 5: Route existing verifier generation through the kernel.** The existing path calls the evaluator and may subsequently render its established command-bearing shell behavior. Do not change normal implementation verification semantics.
- [ ] **Step 6: Expose commandless eligibility.** A floor is observer-evaluable only when `typed_verifier_command` is absent. Command checks/nested verifiers/residual fail-closed checks return an explicit operational deferral value, never executable text.
- [ ] **Step 7: Verify GREEN and generated-verifier regressions.** Run parity, no-execution, deliverable contract, result validation, and verification contract suites.
- [ ] **Step 8: Prove shared-kernel wiring.** Temporarily bypass the kernel in the existing verifier and require its wiring test to fail; invert one floor predicate and require both verifier and observer parity tests to fail. Restore and rerun GREEN.
- [ ] **Step 9: Independent validator review.** Reviewer proves there is one kernel, observer path cannot execute text, residual structure remains authoritative, and existing runtime verifier behavior is preserved.

**Task verification targets:** `archon-workflow --lib`, `declarative_floor_observer`, existing verification contract filters.

---

### Task 10: Centralize terminal finalization and add the observe-only run-end observer

**Files:**
- Create: `src/command/workflow_live_v2_finalizer.rs`
- Create: `src/command/workflow_run_end_observer.rs`
- Create: `src/command/workflow_run_finalizer_tests.rs`
- Create: `src/command/workflow_run_end_observer_tests.rs`
- Modify: `src/command/workflow_live_v2_script.rs`
- Modify: `src/command/workflow_live_v2_lifecycle.rs`
- Modify: `src/command/workflow_live_v2_script_host_state.rs`
- Modify: `src/command/workflow_live_v2_script_host_events.rs`
- Modify: `src/command/workflow_live_v2_run.rs`
- Modify: launch metadata persistence for observer eligibility snapshot

**Interfaces:**

```rust
pub async fn finalize_run(
    context: &WorkflowFinalizationContext,
    summary: WorkflowTerminalSummary,
) -> WorkflowResult<WorkflowTerminalSummary>;

pub fn observe_acceptance_floor(
    snapshot: &RunEndAcceptanceObserverSnapshot,
) -> WorkflowResult<RunEndObserverOutcome>;
```

Finalizer order is terminal summary → terminal state+`FinalizationRecord` → stable terminal event → `terminal_event_committed` → eligible observer → observer completed/failed. Observer failure never changes the returned terminal summary.

- [ ] **Step 1: Write finalizer ordering RED tests with fault injection.** Add state-before-event, observer-pending-with-state, event-commit-before-observer, orderly retry of pending observer without implementation rerun, and success/failure/cancel central-path tests. Assert durable store state after each injected failure, not callback order only.
- [ ] **Step 2: Write eligibility/authority RED tests.** Add legacy omitted snapshot byte-silence, Expected cannot become silent after deletion/replacement, fixed decomposition never eligible, closed run-kind/status table, observer failure terminal invariance, global enforce cannot promote authority, terminal event precedes every observer event, commandless floor shadow, and exact operational deferrals.
- [ ] **Step 3: Record RED.** Compile root harness, `lifecycle_audit`, `store_events`, and `workflow_live_status_write_coordination`; run exact filters serially.
- [ ] **Step 4: Persist launch-time observer snapshot nonblocking.** Whole chain absent omits the backward-compatible field. Any chain artifact records `Expected` with canonical root/path set and readable portable identity. Snapshot never validates/rejects admission.
- [ ] **Step 5: Centralize every authoritative terminal path.** Move state/event responsibility out of direct `host.emit_terminal_status` and `mark_script_failure` branches into `finalize_run`. Script host can form summary but cannot emit authoritative terminal event.
- [ ] **Step 6: Implement durable finalization order.** Persist state and pending intent first, append stable terminal event, persist commit marker, then run observer. Orderly retry completes pending observer without rerunning implementation. Do not add startup claim recovery.
- [ ] **Step 7: Implement closed observer eligibility.** Only normal implementation completion/accepted/noop/needs-review with `Expected` is eligible. `FixedDecompositionV1`, planning/analysis/saved-script, paused/cancelled/failed/blocked/running are ineligible.
- [ ] **Step 8: Implement hard-coded observe-only commandless evaluation.** Validate Expected chain identity, call shared floor kernel, append run-end shadows/operational records after terminal event, validate residual structure, and defer all command-bearing checks. Never read global gate mode for authority.
- [ ] **Step 9: Verify GREEN and terminal regressions.** Run all finalizer/observer tests plus lifecycle/status/event/generated-script suites.
- [ ] **Step 10: Prove centralization/authority sensitivity.** Restore one direct terminal event call and require central-path test failure; invoke observer before event commit and require ordering failure; consult enforce mode and require authority test failure. Restore and rerun GREEN.
- [ ] **Step 11: Independent lifecycle/observer review.** Reviewer traces every terminal path, omitted legacy bytes, Expected mutation behavior, closed eligibility, immutable authority, command deferral, and absence of R2b claims/recovery.

**Task verification targets:** root `--bin archon`, `lifecycle_audit`, `store_events`, `workflow_live_status_write_coordination`.

---

### Task 11: Add synthetic and external proof harnesses

**Files:**
- Create: `tests/workflow_decomposition_synthetic_live.rs`
- Create: `tests/fixtures/decomposition-synthetic/prd.md`
- Create: `tests/fixtures/decomposition-synthetic/project-template/` scratch-only source/config files
- Create: `tests/workflow_decomposition_external_prd_live.rs`
- Create: `tests/workflow_decomposition_proof_support.rs`

**Interfaces:**

The external harness reads only generic runtime inputs:

```text
ARCHON_R2A_EXTERNAL_PRD
ARCHON_R2A_EXTERNAL_TASK_ROOT
ARCHON_R2A_PROTECTED_ROOT
ARCHON_R2A_EVIDENCE_ROOT
ARCHON_R2A_SYNTHETIC_CLEARANCE
```

Both tests are ignored/manual. The external harness refuses unless the synthetic clearance artifact binds the same deployed binary/script/catalog identities.

- [ ] **Step 1: Write a generic synthetic fixture.** One generic PRD produces two or three trivial canonical tasks and scratch-only outputs. Host code serializes a commandless floor exemplar for an artifact outside task write ownership; no model-authored exemplar or domain identifier is committed.
- [ ] **Step 2: Write synthetic lifecycle assertions.** Cover fixed run identity; observe launch; A/B/C/D/E; pause during author dispatch; unchanged logical attempt; status/log inspection; resume skip only by receipt/postcondition; full normal v3 implementation; state/event-before-observer; unmet commandless floor shadow; enforce authority probe unchanged terminal; no Bash/undeclared capability; no secret/candidate content in log.
- [ ] **Step 3: Resolve the existing normal v3 implementation command from CLI parser/help.** Encode that existing surface in the harness. Do not add or invent a second implementation launcher.
- [ ] **Step 4: Write external decomposition-only assertions.** Snapshot protected bytes before/after, require observe mode, run only decompose/status/pause/resume/status, collect all receipts/envelopes/events/log/shadows/reconciliation/terminal evidence, and assert no implementation run, gate promotion, service restart, push, or protected mutation.
- [ ] **Step 5: Add proof preflight refusal tests.** Missing synthetic clearance, mismatched binary/script/catalog, active workflow/Cargo, missing runtime path, non-fresh target, or protected snapshot failure refuses before launch.
- [ ] **Step 6: Record RED without live provider use.** Compile both ignored integration targets with `--no-run`; run only deterministic harness unit/setup filters. Do not execute ignored live tests yet.
- [ ] **Step 7: Implement proof support around existing command surfaces.** Parse run IDs/status, wait by durable state with bounded phase watchdogs, never infer success from silence, and emit a content-addressed evidence manifest.
- [ ] **Step 8: Verify deterministic GREEN.** Run fixture validation, command-surface parsing, preflight refusal, protected-byte comparison, evidence-manifest, and synthetic floor exemplar validation filters.
- [ ] **Step 9: Independent proof-harness review.** Reviewer confirms synthetic-before-external gate, external decomposition-only boundary, no hardcoded external identifiers, no protected writes, no implementation launch, complete evidence, and bounded monitoring.

**Task verification targets:** `workflow_decomposition_synthetic_live --no-run`, `workflow_decomposition_external_prd_live --no-run`; deterministic exact filters only before deployment.

---

### Task 12: Run focused confidence, adversarial reviews, and create the local R2a commit

**Files:** all reviewed R2a paths from Tasks 1–11; no protected path.

- [ ] **Step 1: Freeze implementation ownership.** Stop subagent edits. Recompute protected hashes and compare all non-protected changes with the planned file list. Reject unrelated files.
- [ ] **Step 2: Run serialized focused compile/direct-harness matrix.** In dependency order run `archon-workflow --lib`, `archon-core --lib`, `archon-tui --lib`, root `--bin archon`, then each R2a integration target: `host_command_contract`, `decomposition_state`, `declarative_floor_observer`, `v2_agent_adapter`, `v2_result_contracts`, `v2_resume`, `store_events`, `task_set_contract`, `task_skeleton`, `task_set_edges`, `lifecycle_audit`, `workflow_live_status_write_coordination`, `workflow_live_prompt_contract`, and both proof targets' deterministic filters. Compile one target at a time and execute its recorded harness directly.
- [ ] **Step 3: Run all call-site sabotage cycles.** Raw `w`, persisted HostCommand, publication-after-process, Phase A/B/C/E ordering, raw provider adapter, skill host delegation, shared floor kernel, central finalizer, observer authority, and legacy admission must each fail under sabotage and pass after exact restoration. Recompile after source restoration; do not trust preserved mtimes.
- [ ] **Step 4: Run adjacent package confidence serially.** Execute complete unchanged harnesses for `archon-workflow --lib`, `archon-core --lib`, `archon-tui --lib`, and root `--bin archon` with bounded watchdogs and documented known protected failures only. Do not start with `cargo test --workspace`.
- [ ] **Step 5: Run guarded formatting/lint.** Execute package-scoped `cargo fmt --check`, `git diff --check`, `cargo clippy -p archon-workflow -p archon-core -p archon-tui --all-targets -- -D warnings`, and `cargo clippy -p archon-cli-workspace --bin archon -- -D warnings`, all under coordinator preflight. Never autoformat the protected crate.
- [ ] **Step 6: Request independent review lanes.** Command/process security; persistence/publication; gate/policy/legacy silence; lifecycle/observer authority; UI/status; proof completeness. Resolve every Critical/Important finding and rerun affected exact plus adjacent tests.
- [ ] **Step 7: Verify final scope.** Require no protected path in `git diff --name-only`, no domain identifier in production/generic harness, no R2b type/module, no sabotage marker, no unreviewed file, and no active workflow/Cargo process.
- [ ] **Step 8: Stage explicit reviewed paths only.** Use one `git add <exact path list>` command assembled from the reviewed diff. Inspect `git diff --cached --name-status` and `git diff --cached`; unstage anything unexpected.
- [ ] **Step 9: Create the local implementation commit.** Commit with:

```text
R2a: add engine-native decomposition critical spine

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
```

- [ ] **Step 10: Verify committed tree identity.** Record HEAD, ensure only protected trading WIP remains outside the commit, and bind the release/proof evidence to this exact revision. Do not push.

---

### Task 13: Build and atomically deploy the reviewed commit

- [ ] **Step 1: Re-run full preflight.** Require no active Archon workflow/decomposition/proof/TUI, no Cargo/rustc/linker/test process, exact protected hashes, exact committed non-protected tree, and trusted provider route available without assuming a port.
- [ ] **Step 2: Build once after commit.** Under a 60-minute compile watchdog and one build job run:

```bash
env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 \
  cargo build --release --bin archon
```

- [ ] **Step 3: Verify release provenance before copy.** Prove embedded revision equals HEAD, hash the built binary, and record source manifest/commit/toolchain/build command. Stop on mismatch.
- [ ] **Step 4: Prepare sibling temporary files and atomically deploy.** Copy the verified binary to temporary siblings, fsync where supported, then atomically rename over `/Volumes/Externalwork/archon-cli/project-1/archon` and `/Users/stevenbahia/.local/bin/archon`. Do not deploy while any run is active.
- [ ] **Step 5: Verify both installations.** Require equal SHA-256, equal embedded revision/version, executable health, unchanged protected hashes, and protected services still active. No proof starts until this gate is green.

---

### Task 14: Execute proof package 1 — synthetic full lifecycle

- [ ] **Step 1: Create a clean scratch Git project outside protected paths.** Materialize the committed generic fixture, fresh empty task root, scratch output root, commandless floor exemplar, and evidence directory. Record initial content manifest.
- [ ] **Step 2: Prove no Cargo/workflow conflict.** Run the exact workflow/process inventory, require no Cargo/rustc/test operation, verify both deployed binaries, and start a proof watchdog with approximately 20-second phase heartbeats.
- [ ] **Step 3: Launch first-class decomposition under observe.** Use `archon workflow decompose --prd <scratch-prd> --tasks <scratch-task-root> --yes`; capture run ID immediately and prove run metadata exists before first executor progress.
- [ ] **Step 4: Pause during a model author call.** Wait for durable `ModelCallInFlight`, issue canonical pause, inspect status and `.decompose.log`, and record logical attempt. Require child cleanup/reaping.
- [ ] **Step 5: Resume and complete decomposition.** Issue canonical resume, prove same logical attempt restarted, accepted receipt-backed subjects skipped, and phases A–E complete with exact receipts/envelopes/reconciliation. Stop on any operational failure rather than weakening findings.
- [ ] **Step 6: Host-assert the frozen acceptance criterion.** Before implementation launch, require it exactly matches the serialized commandless floor exemplar and has no `Command` or nested verifier. Refuse proof otherwise.
- [ ] **Step 7: Launch generated scratch tasks through the existing normal v3 implementation surface.** Record implementation run ID and complete the normal lifecycle without any protected path.
- [ ] **Step 8: Prove finalization and observer.** Require terminal state persistence, then terminal event, then observer events; unmet external commandless floor creates shadow evidence and does not alter terminal implementation outcome.
- [ ] **Step 9: Run focused authority probe under global enforce.** Do not launch decomposition under enforce. Re-run only the synthetic observer authority scenario and prove the same terminal outcome plus observe-only shadow.
- [ ] **Step 10: Audit evidence and safety.** Require no Bash/undeclared decomposition capability, no candidate bodies/secrets/env values in `.decompose.log`, no leaked child, no protected mutation, no service changes, and a content-addressed evidence manifest.
- [ ] **Step 11: Independent synthetic clearance review.** A reviewer validates the complete package. Persist clearance binding deployed binary, HEAD, script digest, catalog digest, fixture digest, and evidence manifest. Do not proceed on unresolved Critical/Important findings.

---

### Task 15: Execute proof package 2 — external PRD decomposition only

- [ ] **Step 1: Require synthetic clearance identity.** The harness must verify clearance matches current deployed binary, HEAD, fixed script, and catalog. Any mismatch returns to build/synthetic review; never waive it.
- [ ] **Step 2: Snapshot protected state and external inputs.** Record path+SHA-256 for every protected modified/untracked file, target task root contents, PRD digest, provider route provenance, services, and starting evidence root. Require the intended task root is fresh or R2a run-owned; never adopt a pre-existing chain silently.
- [ ] **Step 3: Prove no Cargo/workflow conflict.** Require exact process guard zero, no Cargo/rustc/linker/test harness, no active workflow/decomposition/proof/TUI, and matching deployed binaries.
- [ ] **Step 4: Launch through the first-class TUI surface under observe.** The reviewing operator launches and monitors `workflow decompose --prd <external-path> --tasks <external-root>`. Record run ID/kind/template/binary/script/catalog identities before author work.
- [ ] **Step 5: Deliberately pause and resume.** Pause during a model call, prove attempt number unchanged and trusted children reaped, inspect durable status/log, resume, and prove valid predecessor/subject skips require receipts and postconditions.
- [ ] **Step 6: Complete only decomposition phases A–E.** Collect acceptance/skeleton provenance, every body attempt and authoritative per-file envelope, set lint/requirements envelopes, exact shadows/remediation, reconciliation, and terminal decomposition state. Operational failure stops the proof; policy findings remain evidence under observe.
- [ ] **Step 7: Prove the implementation boundary.** Assert no implementation workflow/run was created or launched, no generated task executed, no gate mode/promoted authority changed, and `FixedDecompositionV1` created no observer intent.
- [ ] **Step 8: Recompute protected and service state.** Require byte-identical protected WIP, unchanged source tree under the protected crate, no lost untracked file, no unexpected target/repository write, no leaked child, and all protected services still active.
- [ ] **Step 9: Assemble the external evidence package.** Include the 14 spec items: run identity/digests, acceptance, skeleton, pause/resume skip, every body/lint, set gates, shadows/remediation, reconciliation/population, terminal state, log, R1 JSONL, false-positive/unresolved classification, unchanged mortal admission evidence, and protected before/after manifest.
- [ ] **Step 10: Independent evidence review and mandatory stop.** Deliver both packages. Do not push, trigger CI, promote gates, begin R2b/R3/R4, alter protected trading work, or launch any trading implementation. Wait for explicit R2a evidence clearance.

---

## Final Self-Review Checklist

Before implementation approval, the coordinator must confirm:

- [ ] Every R2a scope bullet in spec lines 64–78 maps to Tasks 1–15.
- [ ] Every R2b item in spec lines 80–99 is absent from implementation tasks and named only as a prohibition/handoff.
- [ ] Every stop condition in spec lines 955–977 appears in global constraints, coordinator protocol, or proof gates.
- [ ] Every new public type/function has one defining task and all consumers use the same exact name/signature.
- [ ] Every load-bearing integration has a call-site sabotage, not only a helper behavior test.
- [ ] Every Rust verification names its package/target and is coordinator-owned/serialized.
- [ ] Synthetic proof precedes external proof; external proof is decomposition-only.
- [ ] Build occurs once after the reviewed local implementation commit; both deployed binaries bind that commit.
- [ ] No step stages, commits, formats, runs, or writes `crates/archon-trading`.
- [ ] Work ends at R2a evidence review with no push/CI/promotion/R2b/trading implementation.
