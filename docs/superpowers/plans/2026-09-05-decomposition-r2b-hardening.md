# Decomposition R2b Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Execution is inline with one coordinator, as selected by the operator; do not dispatch subagents. Steps use checkbox syntax for tracking.

**Goal:** Deliver executable native scratch acceptance checks first, then add crash-safe persistence and explicit predecessor adoption to the proven v3 decomposition spine, without promoting gates or starting external task implementation.

**Architecture:** The persisted v3 call/result store remains execution truth. One durable transaction/recovery kernel supports state, publication and projection; host-owned OS locks fence competing writers. A separate, fail-closed acceptance-world adapter executes command-bearing checks only after terminal persistence. Neither feature introduces another scheduler or validator.

**Tech Stack:** Rust 2024; existing Tokio, serde, BLAKE3, SHA-256, fd-lock and libc; QuickJS v3 runtime; native macOS execution in observation-owned scratch roots. No provider/model change.

**Spec:** `docs/superpowers/specs/2026-08-27-decomposition-r2-engine-native-design.md`, especially §§R2b, predecessor adoption, publication, author accounting, finalization, acceptance execution and legacy tripwires. Repository inspection baseline: `0c81c4321`.

**Controlling approved amendment:** `docs/superpowers/specs/2026-09-06-r2b-acceptance-execution-amendment.md`. Native scratch-root execution supersedes prior acceptance backend plans. This revision is resubmitted before implementation; Tasks 1–8 and 11 remain behind separate approval.

**Evidence review:** `docs/superpowers/r2a-evidence-review-2026-09-05.md`. Both functional packages verified. Operator explicitly accepted the historical compiled-input provenance exception on 2026-09-05. This is not a same-revision synthetic proof, a claim of remote judge determinism, or a waiver for future builds. Independent implementation review remains a release gate.

## Global Constraints

Governing invariants, with acceptance execution replaced by the approved amendment:

- “Models never author executables, argv, cwd, environment, catalogs, locks, pins, receipts, or script structure.” Model command declarations remain frozen input to the scratch check runner, never authority over roots, environment, limits or the source commit.
- Unfrozen/unvalidated model command text never executes. Only exact pinned, judged command bytes run natively through the host-owned scratch executor; live roots are never selected as cwd.
- “Host stages remain first-class persisted calls visible to status/resume.”
- “Existing gate commands remain the only artifact validators.”
- “Policy findings do not block in observe; operational/integrity failures always stop the affected phase.”
- “Legacy admission stays freeze-unaware.”
- “Run-end authority remains `ObserveOnly` until R4 promotion.”
- “Protected trading implementation remains untouched and no trading implementation run is launched.”
- “No push or CI trigger occurs.”

Additional execution constraints:

- Every changed Rust file must have **fewer than 500 lines**. Current pressure points: `workflow_host_command_exec.rs` 490, `workflow_decompose.rs` 432, workflow `store.rs` 441. Put new responsibilities in child modules; preserve public reexports.
- No Cargo until `ps -Ao comm | grep -c '^\./archon'` prints zero. Inspect Cargo/rustc/linker/test processes too; one coordinator and one Cargo invocation at a time.
- Never kill or reconfigure protected services. No live proof or deployment in this planning task.
- Stage explicit paths only; never stage protected WIP. Force-add the ignored ledger/document paths individually. Commit before compilation; build/revision checks refer to that commit.
- Red tests before behavior changes; sabotage **production call sites**, not just helpers. Use subprocess crash tests, not only injected `Err` or Drop behavior.
- No PRD-specific logic, path literals, identifiers or domain terminology in production code, comments or generic fixtures.
- Keep ordinary API signatures and legacy serialized fields working. New stored fields are optional or versioned; corrupt present state never falls through as legacy-absent.
- Do not execute any frozen real check during planning/review. Generic approved test commands run only through the native scratch executor.

## Scope, order and approval

This is one R2b release plan with separately reviewable units, not permission to implement them now.

Task IDs are stable review anchors, not execution order. Execute in this order:

```text
FIRST RELEASE SLICE — executed acceptance, existing single-owner R2a finalizer
9 native scratch executor: recorded-commit worktree, copied project, warm target, audit
  → 10 route Command / nested verifier / residual through that world
  → 12A independent acceptance-slice verification and approval STOP

LATER HARDENING — separately approved continuation, no redesign of first slice
1 durable transaction kernel → 2 generation-CAS + atomic events
  → 3 canonical root/executor ownership → 4 publication recovery
  → 5 executable snapshots / trusted-host-command guardian
  → 6 author dispatch ledger → 7 progress outbox → 8 observer crash recovery
  → 11 explicit predecessor adoption → 12B full R2b release review STOP
```

Tasks 9–10 use today's `RunEndObserverContext`, terminal-before-observer ordering,
launch snapshot and run store. They do **not** depend on Tasks 3, 5 or 8. The first
slice retains one supported observer/executor, no deployment while running and no
automatic crash re-execution of command checks. It still needs a narrow world
supervisor, parent-death teardown and verified input export: those are part of
Task 9, not deferred until the general crash-hardening programme. An OS observer
execution lock prevents duplicate world launches, without implementing Task 8's
recoverable claim state machine. After an ambiguous interrupted observation, stop
for reconciliation rather than claiming exactly-once or automatically repeating
service operations.

Task 12A gates only the acceptance slice. Passing it is **not** full R2b closure,
authorization to implement the external task set, or R3/R4 promotion. Task 11 stays
in the programme; the review's shorthand “then 1–8” does not drop adoption.

The native execution amendment is approved. These revised sections are resubmitted
for plan approval; no native executor implementation or real check launch occurs now.

TD-034 is not quietly folded in: fixing skeleton/body mechanical-attempt charging is a separately tracked author-loop change. Task 6 preserves existing logical-attempt policy and records it correctly across crashes. TD-012 healthy terminal-label variability likewise does not justify altering terminal outcomes in a persistence task.

## Verification and clean-source build discipline

Use a clean approved execution checkout for future release builds; do not stash, reset, copy over, or commit protected WIP. If an isolated checkout is needed, obtain worktree approval through the native tool. A source archive may instead be created from the selected commit outside the protected project, with a recorded source manifest and explicitly validated build revision mechanism. A bare archive lacking `.git` cannot satisfy the current `build.rs` revision embedding by itself.

Before each Cargo invocation:

```bash
n=$(ps -Ao comm | grep -c '^\./archon' || true)
printf 'live_archon=%s\n' "$n"
test "$n" = 0 || exit 77
ps -Ao pid,ppid,etime,%cpu,comm | grep -E 'cargo|rustc|rust-lld|ld64|/deps/' || true
export CARGO_TARGET_DIR=/private/tmp/archon-r2b-target
export CARGO_INCREMENTAL=0
export CARGO_BUILD_JOBS=2
```

The process listing is a decision gate, not an invitation to run Cargo regardless of its output. Do not kill unrelated processes. Prefer `cargo test --no-run --message-format=json`, then execute the emitted harness; refuse a filter selecting zero tests. Save full exit codes before printing log tails. Every task's test commit precedes its red compile, and its implementation commit precedes its green compile. Temporary sabotage must be restored before release, with a fresh passing test on the restored tree.

### Plan interpretation

The Rust blocks define proposed interfaces and representative red cases, not an already implemented patch. The syscall and recovery procedures specify implementation obligations; every named integration must be verified in the execution checkout. The loaded-executable handshake and native audit/teardown tests are explicit verification gates: if the chosen platform cannot prove them, stop and return the design decision rather than ship a weaker approximation.

### Shared test conventions

New workflow integration tests use a generic fixture:

```rust
fn spec() -> archon_workflow::WorkflowSpec {
    archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
        name: "durability-proof".into(), task: "prove recovery".into(),
        target_repository_root: None, max_parallelism: 1, max_agents: 1,
        stages: Vec::new(), permissions: Default::default(), learning_hooks: Vec::new(),
    }
}
```

Keep crash-control utilities in `tests/support/r2b_process.rs` (new), shared by the new integration targets. It launches its own test executable with `--exact r2b_child --ignored --nocapture`, supplies fixture root and an operation through test-only environment variables, waits for an acknowledgement over a pipe, then signals only the spawned child. Use barriers/pipes instead of timing sleeps. `r2b_child` dispatches the same production entry points as the parent test; never duplicate journal, identity or validator rules in the driver. Bound process waits and pipe reads. Do not add a production environment-controlled kill switch.

## Task 9 — Native host scratch executor

**Execution order: FIRST. Status: revised for approval, not implemented.**
Authority: approved `2026-09-06-r2b-acceptance-execution-amendment.md`.
Depends on today's terminal snapshot, observer entry point and single-owner run
store, not Tasks 1–8 or 11. Keep the `AcceptanceWorld` port name for compatibility;
its implementation is native scratch-root execution, not a hostile-code sandbox.

**Files**
- Create `crates/archon-workflow/src/acceptance_world.rs`: host-approved command
  reference, outcome and asynchronous execution port.
- Create `crates/archon-core/src/config/sections_acceptance_execution.rs`; connect
  its strict optional config through `sections_workflow.rs` and existing reexports.
- Create small host modules `src/command/acceptance_scratch_policy.rs`,
  `acceptance_scratch_roots.rs`, `acceptance_scratch_inventory.rs`,
  `acceptance_scratch_build.rs`, `acceptance_scratch_executor.rs`,
  `acceptance_scratch_supervisor.rs` and focused sibling test modules.
- Create `tests/r2b_acceptance_scratch.rs` with generic build, data, local-service,
  audit and subprocess fixtures under `tests/fixtures/acceptance-scratch/`.
- Integrate through existing module registries, not a second workflow scheduler.

**Consumes:** implementation run's recorded commit and canonical repository;
launch-bound host policy; existing validated chain/pin and `TrustedCwd`;
`RunEndObserverContext` after terminal persistence.
**Produces:** one observation-owned scratch workspace and native, bounded command
results with live-root hash/audit and teardown evidence. No writes copied back.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcceptanceCommandKind { Command, NestedVerifier, ResidualFailClosed }
#[derive(Clone, Debug)]
pub struct FrozenCommandRef {
    pub acceptance_id: String,
    pub kind: AcceptanceCommandKind,
    pub chain_digest: String,
    pub command_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionDisposition {
    Exited(i32), TimedOut, OutputLimited, ScratchLimitExceeded,
    SetupFailed, ChainIntegrityFailed, LiveRootsChanged, TeardownFailed,
}
#[derive(Clone, Debug)]
pub struct ScratchResult {
    pub disposition: ExecutionDisposition,
    pub stdout: Vec<u8>, pub stderr: Vec<u8>,
    pub live_roots_unchanged: bool, pub teardown_verified: bool,
    pub source_commit: String, pub source_manifest_digest: String,
    pub policy_digest: String, pub evidence_path: std::path::PathBuf,
}
#[async_trait::async_trait]
pub trait AcceptanceWorld: Send + Sync {
    async fn execute(&self, command: FrozenCommandRef)
        -> crate::WorkflowResult<ScratchResult>;
}
```

The host resolves the reference back to validated frozen bytes, never accepts
replacement command text from a caller/model. The adapter owns the observation
workspace; its final cleanup completes before any provisional pass is finalized.

### 9.1 Validate authorization and capture host policy

- [ ] Reuse the existing structure, chain, pin, host-policy and judge validations
  before spawning anything. Re-read command bytes from the pinned chain and match
  chain/command digests; a mismatch is `ChainIntegrityFailed`, with zero shell calls.
  Do not rejudge or alter freezes during observation. A refuted/invalid check does
  not become executable merely because it was published in observe mode.
- [ ] Raw residual-file presence or `validate_residual_gaps` alone is not execution
  authority: that kernel validates fields/permissions but supplies no judgment or
  cryptographic binding of `fail_closed_check`. Require the approved amendment's
  existing pinned, judged command binding; if absent, record integrity/authorization
  failure and do not execute. Do not mint a new freeze/pin/judge mechanism in Task 9.
  This limitation does not affect the eleven frozen Command checks; Task 10 tests
  valid binding and missing-binding refusal separately.
- [ ] Config owns scratch parent, repository identity, declared project input
  directories, combined/separate view, native toolchain PATH, credential-free Cargo
  seed, nonsecret env allowlist, timeout/output/scratch budgets and warm-cache policy.
  Commit comes from the implementation run's recorded final repository revision,
  not current mutable HEAD or model prose. Missing commit refuses setup.
- [ ] Policy starts from no inherited environment. Reject unknown config keys,
  secret/token variables, live-root scratch destinations and copy rules escaping
  declared inputs. Hash effective nonsecret policy; do not log secret values.
  Ordinary admission and LegacyAbsent runs remain unaware of this configuration.

### 9.2 Worktree, project copy and combined view

- [ ] Create an observation-owned detached worktree with a host-built argv:
  `git worktree add --detach <scratch>/repo <recorded-commit>`.
  No stash/reset/clean on the live checkout, no protected WIP copied into source.
  Verify worktree HEAD and tracked source manifest against the recorded commit.
- [ ] Copy only host-declared project directories into `<scratch>/project` at the
  original relative paths, with no-follow regular-file traversal and a content
  manifest. Refuse sockets/devices/FIFOs or escaping symlinks; do not copy workflow
  credentials/config, tokens or host HOME. Required missing inputs are reported,
  never filled with fabricated data. Task files, frozen chain and pin copies are
  read-only and revalidated; scratch chmod is not described as OS security.
- [ ] Default separate view maps ProjectRoot to scratch project, RepoRoot to scratch
  repository. In explicit `project_repository_view="combined"`, populate project
  cwd from the worktree's recorded source and declared project input copies.
  Record ownership of every path; conflicting nonidentical files refuse. Both cwd
  choices must build the same source and see the intended copied inputs. Do not
  rewrite frozen `--target .` or infer cwd from the word cargo.
- [ ] Build revision lookup must resolve the recorded commit in combined view too:
  use a host-created repository metadata reference valid for the scratch worktree,
  or a copied tracked-source view with equivalent verified Git metadata. Never
  stamp a new unrelated scratch commit or silently report unknown provenance.
- [ ] Audit only the input surface used to construct scratch: source paths enumerated
  from the recorded commit, declared project inputs minus operator exclusions, and
  the task root. Do not walk untracked build output, VCS internals, or workflow-store
  output. Nonregular objects are never followed/read as input bytes. Missing or
  changed previously audited input files invalidate observation. Worktree registration
  removal is verified independently, not by hashing `.git`. Unrelated workflow and
  daemon progress must not void a check. Preserve before/after input manifests.

### 9.3 Native build cache and stripped environment

- [ ] Start the child with `env_clear()`, then set a closed host-owned environment:

```text
PATH=<native host toolchain executable paths>
HOME=<scratch>/home
TMPDIR=<scratch>/tmp
CARGO_TARGET_DIR=<scratch>/target
CARGO_HOME=<scratch>/cargo-home
```

  Set Rust toolchain lookup explicitly through verified host policy where needed;
  no shell profiles or inherited provider credentials. Copy Cargo cache contents
  from a configured credential-free seed, excluding credentials and unrestricted
  Cargo configuration. Local services remain reachable normally; do not add network
  restrictions or classify missing credentials as execution containment failures.
- [ ] Create a symlink at the configured relative `target` path in each scratch cwd
  to the same private target directory. Test that `./target/release/...` executes
  what this observation built; no pre-existing live target artifact is reused by
  accident. Command bytes stay unchanged.
- [ ] Share the warm target/cache only within this observation, sequentially, bound
  to identical source/lockfile/toolchain/flags/profile/path inputs. Record cache
  identity before reuse; source changes or target replacement invalidate reuse and
  require a clean rebuild/refusal, never a false fresh result. No cross-run reuse
  of unverified mutable check outputs. Before every check restore the project view
  from the observation's original input snapshot, removing files added by earlier
  checks. Preserve unchanged source mtimes and the warm target/Cargo cache. Record
  per-check mutations and `input_reset=true`; no later check may inherit earlier data.
- [ ] Use the operator's native build resource profile rather than a short-command
  default. Serialize all commands, including those invoking Cargo. The coordinator
  must not compile while a live Archon run executes. Internal post-terminal checks
  require terminal state/event committed and implementation workers finished before
  their Cargo command starts; the idle retained TUI is not an active implementation
  writer. No parallel host build may compete for this observation's target.

### 9.4 Process supervision, root audit and teardown

- [ ] Spawn the host-selected native shell with fixed `-s`, cwd from `TrustedCwd`,
  stripped environment and exact re-derived frozen command bytes on stdin. No raw
  candidate/prose dispatch. Never concatenate bytes into a host launch command.
- [ ] A dedicated observation supervisor owns the child process group and a parent
  liveness pipe; EOF causes termination/reaping even after parent SIGKILL. Children
  cannot inherit the parent-end descriptor and keep it alive. Bound stdin writes,
  concurrent stdout/stderr drain, timeout and scratch size. Full quota walks run
  on a coarse five-second schedule (after the preceding walk finishes), plus one
  final post-exit measurement; cancellation/output checks remain responsive. Record
  the walk count. A cap breach is
  operational regardless of the shell's exit status.
- [ ] On timeout/control/parent death, terminate and reap the owned process group,
  then `git worktree remove --force <owned-worktree>` and remove only this
  observation's scratch. Verify ownership/canonical path before destructive cleanup;
  never generic prune or removal of another worktree. Verify registration gone,
  no live managed group and no remaining owned scratch. Teardown failure cannot pass.
- [ ] Hash the scoped live inputs after all managed children are reaped and cleanup completes;
  any difference is `LiveRootsChanged`, voids all provisional results and preserves
  evidence. Do not automatically undo unexpected live changes or conceal them.
  Test writes only against disposable live fixture roots, never protected real ones.
- [ ] Honest guarantee: scratch construction, stripped env, process-group cleanup
  and before/after audit. This does not prevent malicious absolute-path writes,
  detect transient write-and-restore, deny credentials already held by services,
  or catch every descendant that deliberately leaves the group. Do not claim a
  hostile-executable sandbox. A failing audit detects damage; it does not prevent it.
- [ ] Persist source/policy/command/cwd identities, copied inputs, toolchain/cache,
  budgets, bounded output/status, before/after manifests and cleanup outcome. Local
  provider access has ordinary host semantics; missing credentials or genuine
  provider unavailability appear as the check's ordinary result and diagnostic.
  The observer never changes implementation terminal status.

### 9.5 Tests — production entry points, then sabotage

- [ ] Generic committed fixture builds and executes its real native binary via
  `./target/release/probe --target .`, mutates copied scratch data and leaves live
  source/project/tasks byte-identical. Test separate and combined views, including
  combined project with no original Cargo.toml, and real warm-target reuse on the
  second check. Do not use a fake cargo or a prebuilt unrelated binary.
- [ ] Put a secret canary in parent env and credential seed/config; fixture command
  cannot see it in env or scratch homes. Test allowed nonsecret vars and reachable
  disposable loopback service without forwarding machinery.
- [ ] A frozen fixture command writes directly to its disposable live root and
  exits zero; audit detects the changed hash and final result cannot pass. Tamper
  with frozen bytes or substitute caller command digest: zero process dispatches.
- [ ] Hanging child, inherited-pipe child and output flood hit their limits; killed
  parent leaves no managed group/worktree/scratch. Verify no protected/foreign
  process is signalled. Cleanup fault yields operational evidence. Subprocess tests
  use readiness pipes instead of arbitrary sleeps.
- [ ] Call-site sabotage: bypass command revalidation, change the relative target
  link, omit data-copy mapping, omit env clear, omit final root comparison and omit
  parent EOF cleanup, separately. Each corresponding production-boundary regression
  must fail; restore and rerun before reporting success.
- [ ] Commit test/implementation by explicit paths, every changed Rust file below
  500 lines. Native build only after commit and process gate. No frozen real check
  or external-task implementation is run during plan resubmission.

## Task 10 — Route all command-bearing observer checks through the world

**Files**
- Create `src/command/workflow_run_end_checks.rs`, `workflow_run_end_residuals.rs`, `workflow_run_end_world_tests.rs`.
- Modify `workflow_run_end_observer.rs`, `workflow_live_v2_finalizer.rs`, and existing observer test implementations.
- Reuse `validate_residual_gaps`, `collect_declarative_floor_facts`, `evaluate_declarative_floor` and the existing advanced deliverable kernels; locate their current exports before moving code. Do not create another parser/evaluator.

**Execution order: SECOND.**

**Consumes:** Task 9 `AcceptanceWorld` and its native scratch supervisor, existing immutable terminal snapshot and single-owner R2a finalizer. Task 8 later upgrades crash recovery; it is not a prerequisite.
**Produces:** complete per-criterion evaluation plus coverage records with `ObserveOnly` authority.

- [ ] Convert `WorkflowRunEndObserver::observe` to an async trait method where needed; await it after terminal commit rather than nesting a Tokio runtime or blocking its event loop. Keep context and outcomes unchanged unless an optional versioned count is required.
- [ ] Table-driven red test routing:

```text
Command            → exactly one validated FrozenCommandRef(kind=Command)
Floor, no command  → existing pure floor kernel; zero world requests
Floor, command     → existing floor prerequisites AND world NestedVerifier
Residual record    → only after valid permitted failure AND pinned/judged binding; ResidualFailClosed
LegacyAbsent       → zero probes, zero records, zero world requests
```

Use a recording port to prove all three routes resolve validated pinned command references through the native scratch executor, never a direct shell bypass. Native scratch behavior and audits are proven by Task 9, not that mock.

- [ ] For each world result: `Exited(0)` + unchanged live-root hashes + verified teardown = pass; normal `Exited(nonzero)` = failed criterion; chain-integrity failure, live-root changes, timeout, output/scratch limits, setup failure or unverified teardown = operational observer evidence. None changes terminal status. Never count an unexecuted/operational check as evaluated-and-passed.
- [ ] Residual coverage algorithm:

```text
validate file structure and allowed IDs using existing kernel
reject unknown/supplementary/passing/non-permitted/duplicate criterion coverage
for each permitted failed criterion:
  exactly one authorized record + native scratch fail_closed_check pass → covered shadow
  no record / failed check → uncovered shadow
  operational check failure → operational record, not covered
```

- [ ] Test passing criterion with stale gap, duplicate IDs, forbidden phrase, supplementary coverage, absent gaps, malformed gaps, failed and passing fail-closed checks. Test global enforce leaves terminal bytes unchanged. Frozen identity mutation between launch and observer remains operational.
- [ ] **Sabotage each routing call separately:** Command, nested verifier and residual must each go red when its world dispatch is removed. Leaving the trait/helper in place is not adequate evidence.
- [ ] Commit; run observer/finalizer/residual/legacy suites, then Task-9 generic world tests. No live model invocation needed for these deterministic tests.

## Task 12A — Native acceptance-slice verification and stop

**Execution order: THIRD**, after 9–10 only. Remaining hardening is not authorized.

**Files:** `tests/r2b_acceptance_scratch.rs`, observer routing/authority tests;
readiness and evidence under an operator-selected scratch directory bound to commit.
No PRD-specific logic in engine or generic fixtures; actual check names below are
review-only references to the unchanged frozen contract.

- [ ] Require approval of these revised sections, inline one coordinator, committed
  clean native source and all existing process/staging safeguards. Never overwrite,
  commit or execute protected implementation WIP.
- [ ] Compile root binaries/tests and run native build/data/cache/env/root-audit/
  teardown fixtures; observer/finalizer/legacy/dispatch tests plus all three check
  routing sabotage cases. A normally executed nonzero check is a failed criterion,
  not operational failure merely because it needs a service or credential.
- [ ] Readiness matrix: **all eleven checks are executable by this native design**
  once the recorded implementation commit, configured project inputs and native
  toolchain are available. Executable is not synonymous with passing or already
  verified. Do not maintain a five/six automatic deferral split.

| Frozen check | Native execution requirement | Planned execution |
|---|---|---|
| AC-DL-001 | Combined recorded source/project, build target, copied registry/data | Executable |
| AC-DL-002 | Same build/cwd mapping; ordinary local provider capability access | Executable |
| AC-DL-003 | Native build, scratch temp/input data and scratch ingest output | Executable |
| AC-DL-004 | Native build, copied universe/data inputs, scratch report output | Executable |
| AC-DL-005 | Native build, scratch ingestion/spec/metadata mutation | Executable |
| AC-DL-006 | Native build, scratch malformed-input/metadata cases | Executable |
| AC-DL-007 | Native build and ordinary provider calls; copied inputs/private output | Executable |
| AC-AHDM-001 | Native jq and copied strategy/registry inputs | Executable |
| AC-AHDM-002 | Native file/text tools and copied source artifacts | Executable |
| AC-AHDM-003 | Native build, scratch ingestion/validation/spec mutations | Executable |
| AC-AHDM-004 | Native file/text tools and copied review artifacts | Executable |

- [ ] Populate observed readiness with exact frozen-command digest, source commit,
  cwd mapping, toolchain/cache/profile, copied required inputs and resource limits.
  Missing implementation/data can produce failed criteria; no judge claim substitutes
  for a real execution result. Missing root/commit/toolchain binding is setup failure.
- [ ] Run unchanged real frozen commands only on separate operator authorization;
  capture all eleven results individually without silently skipping provider checks.
  Credentials remain stripped, so credential-dependent criteria may fail visibly.
  No extra deployment or task implementation follows from readiness inspection.
- [ ] Before/after live-root hash equality and verified group/worktree/scratch
  teardown are mandatory for any pass. Observation-induced changes are integrity
  failures. Global enforce still cannot promote observer authority; LegacyAbsent
  stays silent. Independent review examines actual call sites, not only helper tests.
- [ ] Build any approved release from clean committed native source, record source
  manifest/compiler/flags/exit/hash and verify binary revision matches commit.
  This does not retroactively repair historical provenance.
- [ ] Stop for acceptance-slice evidence review. Tasks 1–8, 11 and full 12B retain
  separate approval; no R3/R4 promotion or external-task implementation is implied.

## Task 1 — Durable multi-file transaction kernel

**Files**
- Create `crates/archon-workflow/src/durable_transaction.rs` (record/state machine).
- Create `crates/archon-workflow/src/durable_transaction_io.rs` (no-follow durable I/O).
- Create `crates/archon-workflow/tests/r2b_durable_transaction.rs` and `tests/support/r2b_process.rs` beneath that crate.
- Modify `crates/archon-workflow/src/lib.rs` to export the new module.

**Consumes:** BLAKE3; existing `WorkflowError`, `WorkflowResult`; caller-held writer lock.
**Produces:** one crash-recoverable kernel shared by Tasks 2, 4, 6–8. Journal records contain only host-selected destinations and digests; arbitrary model JSON is never a transaction description.

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PriorDigest { Absent, Blake3(String) }
#[derive(Clone, Debug)]
pub struct Replacement {
    pub relative_path: std::path::PathBuf,
    pub expected: PriorDigest,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RecoveryOutcome { NothingPending, RolledBack, RolledForward }
// RootHandle owns a retained directory fd. No Clone or public raw-fd constructor.
// Transaction functions are methods on that handle, called under its writer lock.
```

Specify `RootHandle::open(root: &Path) -> WorkflowResult<RootHandle>`,
`RootHandle::commit(txn_id: &str, replacements: &[Replacement]) -> WorkflowResult<()>`,
and `RootHandle::recover() -> WorkflowResult<RecoveryOutcome>`.

- [ ] **Red test:** write `out.json = old`, commit with the wrong expected digest, assert error and exact unchanged bytes. Also refuse `../escape`, absolute paths, intermediate symlinks, final symlinks, duplicate destinations and pre-existing hardlinked destination files.

```rust
let tmp = tempfile::tempdir().unwrap();
std::fs::write(tmp.path().join("out.json"), b"old").unwrap();
let root = RootHandle::open(tmp.path()).unwrap();
let replacement = Replacement {
    relative_path: "out.json".into(), expected: PriorDigest::Absent, bytes: b"new".to_vec(),
};
assert!(root.commit("txn-1", &[replacement]).is_err());
assert_eq!(std::fs::read(tmp.path().join("out.json")).unwrap(), b"old");
```

- [ ] Commit test and run `cargo test -p archon-workflow --test r2b_durable_transaction`. The first red may be the absent module/API; after API exists, all refusal/crash tests must fail for the intended missing behavior before implementation is considered covered.
- [ ] **Implement:** traverse directories using `openat(O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC)`, retain fds, and open files with `O_NOFOLLOW`. Validate inode/device and ordinary-file status. Write unique sibling staging/backup names with create-new semantics. Never unlink a lockfile to reclaim ownership. Flush content, journal and affected directories.
- [ ] The journal stores schema, transaction ID, sorted destination/prior/new digests, modes, backup/staged identities and phase. Exact recovery algorithm:

```text
No intent: no destination changes are legal.
Prepared intent durable: original files remain authoritative; restore prior bytes
  from verified backups if any rename occurred before a durable commit decision.
Commit decision durable: verify all staged/new bytes; roll forward every remaining
  destination, then fsync directories, then mark Applied durable.
Applied durable: remove only this transaction's verified backups/staging; retain
  immutable outcome evidence until the caller's corresponding receipt is durable.
Unknown bytes / root identity mismatch: refuse; never infer acceptance or overwrite.
```

- [ ] `txn_id` reuse with a different payload is corruption, not another write. Reader recovery under the same lock precedes every authoritative read in later tasks. Files on different filesystems are **not atomically visible**; the kernel guarantees recovery for cooperating readers, not simultaneous visibility to unrelated processes.
- [ ] **Crash tests:** kill after staged fsync, Prepared fsync, each rename, Commit decision fsync, each directory fsync, Applied fsync and cleanup. Repeat recovery twice; assert old-or-new complete set, never silently mixed. Rename the ancestor path during the test and prove retained handles cannot redirect writes to a sentinel outside the root.
- [ ] **Sabotage:** omit the expected-digest comparison and directory fsync call separately; assert the corresponding test fails. Crash tests must inspect durable journal phases as well as final bytes (a mere normal process exit is not fsync evidence).
- [ ] Commit the implementation; rerun targeted suite. Keep new files under cap.

## Task 2 — Generation-CAS state/checkpoint and atomic event append

**Files**
- Modify workflow `store.rs`, `events.rs`, `lifecycle_a.rs`, `control.rs`.
- Modify `v2/result_store.rs` at `save_checkpoint` / `save_call_record` integration boundaries, preserving non-run/test-store callers.
- Create `crates/archon-workflow/src/run_transaction.rs` and `tests/r2b_run_transaction.rs`.

**Consumes:** Task 1, `.control.lock`, existing generation and `WorkflowV2Checkpoint`.
**Produces:** lock-scoped `RunTransaction` methods for state, call result, checkpoint and stable event IDs. The fixed runtime opts in; legacy standalone store users keep their signatures.

Required interface:

```rust
pub struct CommitRevision {
    pub expected_generation: u64,
    pub expected_storage_revision: u64,
}
// Proposed additional methods on WorkflowStore:
// transact_run<T>(&self, run_id: &str, expected: CommitRevision,
//   f: impl FnOnce(&mut RunTransaction) -> WorkflowResult<T>) -> WorkflowResult<T>
// append_event_once(&self, run_id: &str, event_id: &str,
//   kind: WorkflowEventKind, detail: serde_json::Value) -> WorkflowResult<WorkflowEvent>
```

`storage_revision` is a new host-owned sidecar revision; **do not increment lifecycle generation for every write**. That would cancel healthy running work. `RunTransaction` exposes `state`, `set_state`, `save_call_record`, `save_checkpoint` and `event_once`; these stage a single Task-1 transaction on commit. No nested acquisition of `.control.lock`: locked methods are crate-private and public wrappers acquire once.

- [ ] **Red tests:** two processes load the same generation/storage revision, then commit disjoint state changes. Exactly one succeeds; the loser reloads rather than overwriting the winner. Pause/cancel wins over late dispatch completion.
- [ ] Two processes append 50 events each; require exactly 100 distinct consecutive sequence numbers. Repeating one stable event ID with identical kind/detail returns the original event; changing payload for the same ID refuses.

```rust
let first = store.append_event_once(&run.id, "boundary-1",
    WorkflowEventKind::StageCompleted, serde_json::json!({"event":"boundary"})).unwrap();
let again = store.append_event_once(&run.id, "boundary-1",
    WorkflowEventKind::StageCompleted, serde_json::json!({"event":"boundary"})).unwrap();
assert_eq!(first.seq, again.seq);
assert_eq!(std::fs::read_to_string(store.events_path(&run.id)).unwrap().lines().count(), 1);
```

- [ ] **Implement:** allocate sequence from validated maximum persisted sequence while holding the lock, not from line count. Persist event bytes and dedupe identity through Task 1, fsync append storage, then return. A partial trailing record is recoverable only when the journal proves its interrupted append; malformed committed lines refuse.
- [ ] Route lifecycle state/event transitions and fixed call result/checkpoint commits through the transaction. Add a schema-versioned sidecar defaulting to revision zero only when wholly absent. Preserve `WorkflowRun.generation` semantics and existing serde projections.
- [ ] **Sabotage:** replace production `transact_run` at fixed-call checkpoint with old writes; subprocess crash between result/checkpoint must fail. Remove atomic append lock; competing-process test must fail, not a helper-only test.
- [ ] Commit and run the new target plus workflow control/lifecycle and result-store suites.

## Task 3 — Canonical task-root writer lease and executor fencing

**Files**
- Create `src/command/workflow_task_root_lease.rs`, `workflow_task_root_lease_tests.rs`.
- Modify `workflow_decompose.rs::create_claimed_run`, `workflow_decompose_resume.rs`, `workflow_live_v2_fixed_run.rs` and `workflow_host_command_exec.rs` through a new small child if needed.

**Consumes:** Task 2, host-canonical task root, run ID and lifecycle generation.
**Produces:** `TaskRootLease` retaining an OS exclusive lock and `WriterFence { run_id, generation, epoch, root_device, root_inode }` serialized in mutation intents. Constructor fields are host-owned.

- [ ] **Red test:** launch two separate processes with distinct workflow stores but the same canonical task root (including one symlink alias). Only one obtains the writer lease; neither process creates staged output before obtaining it.
- [ ] **Implement:** stable root-local lockfile (excluded from gate input manifests), opened no-follow, never replaced/deleted; lifetime OS lock, not PID-only liveness. Under the lock compare persisted owner/run/generation and advance epoch for a recovered executor. Paused/cancelled runs keep the **logical reservation** even after their OS executor lease is released. Another run requires explicit reservation release, never timeout alone.
- [ ] Require `WriterFence` at every Task-4 publication and state-commit boundary. Resume of the same run may recover a dead executor only after obtaining the OS lock and satisfying persisted run/identity checks. A second live executor is refused even if metadata claims it is stale.
- [ ] Kill the winning child; a same-run recovery acquires the lock, increments epoch, and rejects the dead child's delayed commit. Do not use PID reuse as identity.
- [ ] **Sabotage:** bypass lease acquisition in the launcher; two-process real launch test must fail.
- [ ] Commit; run new root lease tests and existing decomposition claim tests. Existing signatures remain delegating wrappers for tests and legacy call sites.

## Task 4 — Recoverable publication, receipts and call-record adoption

**Files**
- Create `src/command/workflow_host_command_journal.rs`, `workflow_host_command_recovery.rs`, `workflow_host_command_recovery_tests.rs`.
- Modify `workflow_host_command_publish.rs::publish_audited`, `workflow_host_command_exec.rs`, `workflow_host_command_exec_live.rs`, `workflow_task_set_publish.rs`, `workflow_live_v2_script_host_exec.rs`.

**Consumes:** Tasks 1–3; `AuditedPublication`, host destinations, `HostCommandResult`, current postcondition kernels.
**Produces:** durable `HostPublicationJournalV1` and exact reconstruction of the ordinary `WorkflowV2CallRecord`, not a second result format.

Journal includes call/input/output identity, writer fence, binary/script/catalog hashes, prepared envelope, exit/drain/reap attestations, expected prior/new destination hashes, receipt and postcondition. Credentials are never serialized. A journal missing successful parent-observed completion cannot authorize publication.

- [ ] **Red fixture:** reuse `workflow_host_command_publication_tests.rs` fixtures. A counted fake judge supplies one prepared acceptance result. Kill after durable publication but before saving the ordinary call record. Resume must recover exactly that result and keep judge invocation count **1**.
- [ ] Implement `recover_host_publication(store, run_id, call_id, fence) -> WorkflowResult<Option<HostCommandResult>>`, called **before dispatch** and on resume. It validates identity, completes Task-1 recovery, verifies current bytes, runs existing authoritative postcondition kernels and saves ordinary call/checkpoint evidence through Task 2. It never rejudges a committed freeze.

```text
Journal absent → ordinary dispatch.
Prepared but no commit decision → restore/retain original complete set; do not accept.
Committed matching output → finish publication/receipt/call checkpoint; reuse result.
Applied receipt but changed live bytes → invalidate reuse, report conflict.
Wrong run/epoch/catalog/input → refuse operationally; do not republish or overwrite.
```

- [ ] Preserve `gate-envelope.json` and shadow invocation IDs alongside result recovery. Publish shadow evidence once per invocation, including crashes after output commit but before JSONL append; Task 7 uses the same IDs.
- [ ] Test each artifact/lock/pin boundary, prior destination absent/present, interruption, duplicate recovery, changed bytes and a symlink-swapped ancestor. Never claim group atomicity across filesystems; authoritative readers must recover under the lease before reading.
- [ ] **Sabotage:** delete the live executor's recovery call while retaining the helper; duplicate judge count or missing ordinary record must fail.
- [ ] Commit; run publication, freeze republish, host executor and history-replay regression tests.

## Task 5 — Run-owned executable bytes and independent guardian

**Files**
- Create `src/command/workflow_executable_snapshot.rs`, `workflow_host_guardian.rs`, `workflow_executable_snapshot_tests.rs`, `workflow_host_guardian_tests.rs`.
- Modify `workflow_decompose.rs`, `workflow_decompose_resume.rs`, `workflow_host_command_catalog.rs`, `workflow_host_command_supervisor.rs` and CLI internal dispatch in `src/cli_args/strategy_actions_workflow.rs` / `workflow_decompose_cli.rs`.

**Consumes:** writer fence, Task-1 durable writes, host-resolved `current_exe`.
**Produces:** `ExecutableSnapshotV1 { relative_path, sha256, byte_len, binary_revision, catalog_digest, script_digest }`; a same-executable startup handshake before child reads candidate stdin.

- [ ] **Red tests:** copy executable A into a temporary installation path, launch a run, atomically replace installation with executable B, and invoke another host stage. It must still execute snapshot A; corrupted snapshot bytes must refuse before the candidate reaches any child. Do not overwrite real installed binaries for this test.
- [ ] **Implement:** open source executable once, hash/copy from that fd into a run-owned create-new file, fsync, set executable non-writable mode, record manifest. Retain root/executable identity. Rehash before each spawn and require an internal handshake containing expected snapshot hash/revision/catalog plus a host nonce. Candidate bytes are withheld until handshake succeeds. A process reports its own **loaded executable identity**, not merely re-reading an arbitrary path supplied in argv; on macOS bind the executable vnode/executable identity and validate the supported replacement threat model in a platform test.
- [ ] Cross-version outer launcher must not silently execute old scripts using new code. A mismatch names the verified snapshot-specific recovery command. Automatic delegation, if offered, is explicit host policy and never loads a model-selected executable.
- [ ] **Guardian:** dedicated trusted child owns command spawning and listens on an inherited parent-liveness pipe. EOF triggers teardown/reap independent of the parent destructor. Pipe fds are close-on-exec everywhere except the intended guardian endpoint; command descendants cannot keep the liveness pipe open. Return success only after bounded output/drain/reap and guardian acknowledgement.
- [ ] Kill the parent with SIGKILL while command is running; assert guardian cleans its managed process tree. Also test cancelled/paused generation, timeout, output flood and pipe-held descendant. For trusted host capabilities detaching remains forbidden. Do **not** describe PGID probing as detecting `setsid` escape; Task 9 also promises process-group cleanup and audit, not arbitrary detached-descendant security.
- [ ] **Sabotage:** resolve installed path at the live call site rather than snapshot; A/B replacement test fails. Drop the guardian's EOF branch; parent-death test fails.
- [ ] Commit; run supervisor, resume identity, lifecycle shutdown and new executable tests. A failed platform enforcement probe blocks enabling live-replacement support rather than weakening the claim.

## Task 6 — Crash-active author dispatch ledger and active time

**Files**
- Create `crates/archon-workflow/src/author_dispatch.rs`, `tests/r2b_author_dispatch.rs`.
- Create `src/command/workflow_author_dispatch_store.rs`.
- Modify `workflow_live_v2_client.rs::run_agent_raw_request`, `workflow_live_v2_script_host_exec.rs`, fixed decomposition resume and `workflow_decompose_state.rs`.

**Consumes:** Task-2 transactions, writer fence, raw `WorkflowAgentCall`/`WorkflowAgentOutcome`.
**Produces:** ledger entries keyed by run/call/logical attempt/input hash; no duplicated author-loop policy.

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DispatchPhase {
    Prepared,
    Dispatched { provider_request_id: Option<String> },
    ResultPrepared { outcome_blake3: String },
    Recorded,
    Interrupted,
    Ambiguous,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveTime {
    pub accumulated_ms: u64,
    pub heartbeat_sequence: u64,
}
```

- [ ] **Red tests:** pause/resume does not charge paused wall time; kill after `ResultPrepared` and recover without a second provider call; kill after request dispatch but before durable reply records `Ambiguous`, not Accepted or fresh success.
- [ ] Persist Prepared before provider dispatch, persist Dispatch acknowledgement/request ID when available, save complete raw result bytes durably before parsing or creating a call record, then atomically link result/checkpoint. Persist periodic elapsed **monotonic deltas** and heartbeat sequence; never compare process-local monotonic clock values across boots. A crash may leave an explicitly bounded unaccounted heartbeat interval; charge conservatively from a documented upper bound or refuse automatic deadline continuation, never silently reset the timeout.
- [ ] Reuse requires exact input/attempt/identity and complete durable result; truncation remains truncation. Remote request recovery is used only where the provider already supplies a verifiable request retrieval/idempotency contract. Otherwise ambiguity stops with a named operator choice; it cannot promise provider-side exactly-once.
- [ ] Implementation flow:

```text
resume ResultPrepared → validate digest → ordinary parse/host recording → Recorded
resume Recorded       → ordinary call-record replay
resume Prepared       → no acknowledged dispatch; recover under explicit policy
resume Dispatched     → retrieve same request if supported, otherwise Ambiguous
pause/cancel          → Interrupted, retain logical attempt and active elapsed time
```

- [ ] **Sabotage:** remove production result-preparation write before parser/record save; crash at that boundary must force the test red. Tests assert provider call count and persisted disposition, not just ledger helper output.
- [ ] Commit; run author budget/control/raw-outcome/replay suites. TD-034 stays open unless separately authorized.

## Task 7 — Durable progress outbox and shadow append replay

**Files**
- Create `crates/archon-workflow/src/progress_outbox.rs`, `tests/r2b_progress_outbox.rs`.
- Modify `src/command/workflow_decompose_state.rs`, `workflow_decompose_events.rs`, `workflow_decompose_log.rs`, `workflow_gate.rs` and fixed resume.

**Consumes:** stable event IDs and transactions from Task 2; Task-4 publication invocation IDs.
**Produces:** outbox entries `{ event_id, event_seq, projection_digest, log_line, delivery_state }` with no candidate content or credentials. Transient TUI delivery remains best-effort after durable evidence.

- [ ] Red test: kill after event commit, before `.decompose.log`; resume produces one log row with the original sequence. Kill after log fsync but before outbox acknowledgement; resume still produces one row. Same invocation cannot duplicate shadow JSONL.
- [ ] Commit the event and outbox entry together. Projection derives from stored call/event evidence; no fabricated new progress sequence on replay. Append under an OS lock, validate/tolerate only journal-proven torn tails, and dedupe stable IDs before acknowledging delivery. Repeated ID with different bytes is operational corruption.
- [ ] Ordering:

```text
ordinary result commit → event/outbox commit → log/shadow fsync → ack commit → TUI
```

- [ ] Do not dedupe by prose, timestamp or event label alone. Two identical findings from different invocations remain two records; the same invocation replays once. Rebuild derived projections from durable events only when schema/identity matches.
- [ ] Sabotage the resume drain call: crash-before-log test fails. Sabotage append dedupe: crash-after-log test fails.
- [ ] Commit; run progress/log/state and ordinary status tests. Legacy runs produce no fixed-decomposition log.

## Task 8 — Exclusive observer claims and finalization-only startup recovery

**Files**
- Modify `crates/archon-workflow/src/v2/finalization.rs` with backward-compatible version handling.
- Create `src/command/workflow_finalization_recovery.rs`, `workflow_finalization_recovery_tests.rs`.
- Modify `workflow_live_v2_finalizer.rs`, `workflow_finalization_status.rs`, `workflow_run_end_observer.rs` and explicit workflow status/resume startup entry points.

**Consumes:** Tasks 2, 3, 7 and the already delivered Tasks 9–10; existing `RunEndObserverContext`, `FinalizationRecordV1`, `WorkflowRunEndObserver`. Upgrade the first-slice narrow execution lock rather than adding a conflicting observer owner.
**Produces:** exclusive observer owner token/epoch plus durable result-prepared phase; a recovery operation that performs finalization only.

- [ ] **Red tests:** two processes attempt finalization of the same terminal run; exactly one observer executes. Kill after terminal state before event, after event before claim, during observation, after observer result preparation, and before completion acknowledgement. Recovery never reruns implementation calls.
- [ ] Extend the finalization record with optional claim/result identity, or introduce an explicitly migrated V2 record; absent legacy observer snapshot stays silent. Read/load and claim under one lock; release the lock before expensive observation but retain the OS claim handle. A dead claim can be recovered only by successful OS acquisition and epoch advancement, not elapsed timestamp alone.
- [ ] Recovery decision table:

```text
No finalization + legacy-absent → no observer action or filesystem probe
Terminal state missing        → recover only a journal-proven terminal transition
Terminal event missing        → append same stable terminal event once
Observer pending/dead claim   → claim exclusively; evaluate eligible snapshot
Observer result prepared      → publish that result, no duplicate evaluation
Observer completed/failed     → no automatic duplicate invocation
```

- [ ] Verify terminal status remains byte-equivalent under global enforce. Snapshot removed/replaced after launch produces observer operational evidence, never silent LegacyAbsent. Failed/blocked/cancelled non-eligible implementations do not run acceptance checks.
- [ ] Status may report pending recovery without launching providers. An explicit resume/finalize recovery entry point acquires ownership and does finalization only. Do not turn every `status` call into command execution.
- [ ] Sabotage claim acquisition at `finalize_summary`: two-process test fails. Sabotage finalization-only branch: provider counter must become nonzero and fail.
- [ ] Commit; run finalizer/observer/legacy snapshot and authority-probe suites.

## Task 11 — Explicit non-mutating predecessor adoption

**Files**
- Create `crates/archon-workflow/src/adopted_predecessor.rs`, `tests/r2b_adopted_predecessor.rs`.
- Create `src/command/workflow_predecessor_adoption.rs`, `workflow_predecessor_adoption_tests.rs`.
- Modify `workflow_decompose.rs`, `workflow_decompose_resume.rs`, `workflow_decompose_cli.rs`, `src/cli_args/strategy_actions_workflow.rs`, parser tests and the existing TUI decompose argument parser (find via `WorkflowAction::Decompose`, not a new parallel parser).

**Consumes:** Tasks 2–5; `validate_acceptance_bundle`, `validate_full_chain`, current gate policy, exact persisted chain/pin/shadow bytes.
**Produces:** `AdoptedPredecessorReceiptV1`, opt-in launcher policy, and normal v3 persisted import evidence. Existing launch functions delegate with adoption disabled.

Receipt schema fields:

```rust
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AdoptedPredecessorReceiptV1 {
    pub schema_version: u32,
    pub canonical_task_root: String,
    pub artifact_blake3: std::collections::BTreeMap<String, String>,
    pub validation_results: std::collections::BTreeMap<String, String>,
    pub inherited_shadow_blake3: std::collections::BTreeMap<String, String>,
    pub adopter_run_id: String,
    pub binary_sha256: String,
    pub script_digest: String,
    pub catalog_digest: String,
    pub adopted_at: String,
    pub event_id: String,
}
```

The receipt is host-minted only after every required check; maps have a closed required-key set validated by the constructor, not arbitrary author annotations. Full chain artifacts include acceptance/skeleton/locks/pin and their exact predecessor links; incomplete chains refuse. Bind current PRD text/digest and current-binary validator/policy outcomes as well as historical gate provenance. Inherited shadows need original ID/digest evidence; count-only history cannot be laundered into adopted clean provenance.

- [ ] **Red tests:** pre-existing internally consistent chain without explicit opt-in does not skip. Explicit opt-in with exact clean chain writes receipt/event only; every input artifact remains byte-identical. A copied receipt for another run/root or one changed byte rejects reuse.
- [ ] Proposed explicit `--adopt-predecessor` flag routes through a new options-bearing launcher, with old public wrappers defaulting false. Record the choice in launch digest and persisted arguments. Obtain root lease before validating/minting.
- [ ] Do not add a validator-only bypass to the script. Add one **host-authored** persisted import capability/call whose receipt supplies the existing acceptance/skeleton outcomes only after current validation. It cannot be chosen by raw model output. The import has no task-root write set; it writes only run evidence. Reconciliation accepts its typed adopted receipt through a reviewed explicit branch, never by fabricating a publication receipt that says bytes were written.
- [ ] Reuse checks receipt plus current digest equality and ordinary kernels; a receipt's mere existence cannot authorize skipping. Acceptance-only vs full-skeleton adoption must be explicit levels; default implementation adopts a full valid chain only, and partial chains continue to refuse. No silently adopted bodies.
- [ ] **Sabotage:** remove exact-byte revalidation from the launch/import call site; tamper test fails. Remove opt-in check; default pre-existing-chain test fails. Normal legacy workflow admission never calls adoption.
- [ ] Commit; run adoption, parser, freeze-bundle, resume/reconciliation and legacy-admission regression suites.

## Task 12B — Full R2b release verification and stop

Run only after 1–8 and 11 are subsequently complete. This reuses the same review
procedure with the expanded crash/ownership scope; it is not satisfied by 12A.

- [ ] Collect each task's red/green/call-site sabotage evidence and run all new
  subprocess race/crash matrices. Do not substitute normal cancellation for SIGKILL
  or helper tests for a two-process race.
- [ ] Run root `cargo check --bins --tests`, workflow library/integrations, touched
  transport suites, fixed-decomposition/host-command/finalizer/observer/CLI/TUI and
  history-replay tests, and legacy mortal-admission tripwires. Report actual counts
  and baseline failures; never call filtered results a green whole workspace.
- [ ] Re-run acceptance-slice generic scratch/audit regression tests after integrating
  new transaction/claim/snapshot machinery; verify no new world invocation or
  endpoint access during ambiguous recovery.
- [ ] Independent review covers the complete deferred-scope matrix. Build after
  final commit from clean approved source, recording exact compiled-input provenance
  and binary identity. Keep the historical R2a exception narrowly scoped.
- [ ] Record release evidence and verified ledger updates. Any authorized deployment
  uses temp-copy + atomic rename to only requested destinations.
- [ ] Stop for full R2b evidence review. No R3/R4 promotion or external-task
  implementation follows automatically.

## Coverage and plan review checklist

| Approved R2b scope | Tasks / failure evidence |
|---|---|
| Native pinned Command, nested verifier and residual check | First slice 9 → 10 → 12A; native build/data/env/audit/teardown and routing sabotage |
| Explicit predecessor adoption | 11; opt-in/no-mutation/exact-byte/legacy tests |
| Snapshots, handshake, live replacement | 5; A/B executable swap and parent-death tests |
| Canonical writer leases, cross-run CAS, no-follow | 1–4; separate-process writer races and ancestor swap |
| Composite journal and committed-result adoption | 1, 4; every fsync/rename/receipt crash cut, judge count stays one |
| Crash-active author ledger/active-time recovery | 6; pause clock and prepared-result/ambiguous-request cases |
| Generation-CAS and event allocation/append | 2; competing writers, stable event IDs and torn append recovery |
| Progress outbox replay | 7; event-before-log/log-before-ack crashes |
| Observer claims and finalization-only recovery | 8; competing finalizers, every terminal/observer boundary |
| ObserveOnly and legacy behavior | First 10/12A; later 8/11/12B; unchanged terminal state under enforce and no legacy probing |

- [ ] Review revised tasks 9/10/12A against the approved native scratch amendment before implementation.
- [ ] Approve plan execution separately; current task produced planning documents only.
- [ ] Preserve historical R2a provenance exception as scoped; no future relaxation.
- [ ] All helper interfaces above have a named owning task and production integration point.
- [ ] New Rust implementation files are split before 500 lines; tests use the nearest existing fixtures.
- [ ] Independent review and release evidence remain gates, not completed checkboxes.


## Current approval boundary

Approved architecture: native scratch-root execution under the 2026-09-06
amendment. Reordered plan sections 9 → 10 → 12A are submitted for review.
Execute inline with one coordinator only after that approval. Tasks 1–8 and 11
are unchanged in scope and require separate approval. No full R2b closure,
external implementation, deployment or live proof launch is implied.

## R3 implementation handoff — commit before finalization

This is a required entry in the R3 implementation plan, not authorization to run R3.
Task implementations must stage only their owned paths and commit completed work
before finalization records the source revision. The final integration commit must
contain every implementation output intended for acceptance. Uncommitted checkout
changes are deliberately absent from the observer's recorded-commit worktree and
must never be described as verified. Preserve unrelated operator WIP: do not stage
it to satisfy this requirement. Record the implementation commit in the run, then
finalize and observe that exact commit.

Before any PRD implementation workflow: obtain second review of the native slice,
configure the approved project profile, and run the separately authorized frozen-
contract observation against the current committed source without implementation.
Record all eleven actual results; expected criterion failures are not operational
failures. No implementation starts merely because the adapter or dry run completes.
