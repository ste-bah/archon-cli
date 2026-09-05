# Decomposition R2b Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Execution is inline with one coordinator, as selected by the operator; do not dispatch subagents. Steps use checkbox syntax for tracking.

**Goal:** Deliver executable isolated acceptance checks first, then add crash-safe persistence and explicit predecessor adoption to the proven v3 decomposition spine, without promoting gates or starting external task implementation.

**Architecture:** The persisted v3 call/result store remains execution truth. One durable transaction/recovery kernel supports state, publication and projection; host-owned OS locks fence competing writers. A separate, fail-closed acceptance-world adapter executes command-bearing checks only after terminal persistence. Neither feature introduces another scheduler or validator.

**Tech Stack:** Rust 2024; existing Tokio, serde, BLAKE3, SHA-256, fd-lock and libc; QuickJS v3 runtime; proposed Docker Desktop Linux acceptance world on macOS. No provider/model change.

**Spec:** `docs/superpowers/specs/2026-08-27-decomposition-r2-engine-native-design.md`, especially §§R2b, predecessor adoption, publication, author accounting, finalization, acceptance execution and legacy tripwires. Repository inspection baseline: `0c81c4321`.

**Amendment approved with first-slice conditions on 2026-09-05:** `docs/superpowers/specs/2026-09-05-r2b-acceptance-world-amendment.md`. Approval is limited to Linux feasibility first, scratch build/data views, deny-all observed networking and run-owned trusted build-cache seeds. Endpoint allowances are deferred. Tasks 1–8 and 11 require separate approval.

**Evidence review:** `docs/superpowers/r2a-evidence-review-2026-09-05.md`. Both functional packages verified. Operator explicitly accepted the historical compiled-input provenance exception on 2026-09-05. This is not a same-revision synthetic proof, a claim of remote judge determinism, or a waiver for future builds. Independent implementation review remains a release gate.

## Global Constraints

Copied governing invariants:

- “Models never author executables, argv, cwd, environment, catalogs, locks, pins, receipts, or script structure.” Model command declarations remain opaque input to the isolated check runner, never host process authority.
- “Raw model-authored command text never executes on the host.”
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
- Do not execute model output in a shell during review or testing outside the acceptance world.

## Scope, order and approval

This is one R2b release plan with separately reviewable units, not permission to implement them now.

Task IDs are stable review anchors, not execution order. Execute in this order:

```text
FIRST RELEASE SLICE — executed acceptance, existing single-owner R2a finalizer
9.0 pinned Linux release-build experiment (one hour; failure stops execution)
  → 9 host-policy world: input layout + build/data scratch + observed deny-all network
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

The proposed platform/policy amendment is subject to approval. Docker availability
does not prove containment or build compatibility. No image pull/build, guest
command, live endpoint request or host configuration change occurs during planning.

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

The Rust blocks define proposed interfaces and representative red cases, not an already implemented patch. The syscall and recovery procedures specify implementation obligations; every named integration must be verified in the execution checkout. The loaded-image handshake and containment tests are explicit feasibility gates: if the chosen platform cannot prove them, stop and return the design decision rather than ship a weaker approximation.

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

## Task 9 — Executable acceptance world with host-owned scratch and egress

**Execution order: FIRST.** Depends only on the existing R2a terminal snapshot,
observer entry point and run store. Read the approved amendment and conditions first.

**Files**
- Create `crates/archon-workflow/src/acceptance_world.rs` (request/result port).
- Create `crates/archon-core/src/config/sections_acceptance_world.rs`; wire the
  field into `sections_workflow.rs::WorkflowRuntimeConfig` and its existing
  reexports in `sections.rs` / `config/mod.rs` as applicable. Do not edit operator
  configuration while adding the schema.
- Create host modules `src/command/acceptance_world_policy.rs`,
  `acceptance_world_inventory.rs`, `acceptance_world_layout.rs`,
  `acceptance_world_docker.rs`, `acceptance_world_supervisor.rs`,
  `acceptance_world_egress.rs`, and separate test modules below the line cap.
- Create `tests/r2b_acceptance_world_live.rs` (operator-enabled/ignored) and
  `tests/fixtures/acceptance-world/` generic build, data and local-service fixtures.
- Register modules with existing parents. Do not expose a model-callable capability
  that creates images, mounts, environments or networks.

**Consumes:** `RunEndObserverContext`, validated frozen checks, current host-resolved
project/repository/task roots and operator configuration captured at launch.
**Produces:** async isolated execution with exact policy/input/image identity and
explicit distinction between a failed criterion and an execution restriction.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcceptanceCommandKind { Command, NestedVerifier, ResidualFailClosed }
#[derive(Clone, Debug)]
pub struct WorldLimits {
    pub timeout_ms: u64, pub stdout_bytes: usize, pub stderr_bytes: usize,
    pub memory_bytes: u64, pub pids: u32, pub scratch_bytes: u64,
}
#[derive(Clone, Debug)]
pub struct WorldRequest {
    pub invocation_id: String,
    pub kind: AcceptanceCommandKind,
    pub command_stdin: Vec<u8>,
    pub cwd: crate::task_set_contract::TrustedCwd,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldDisposition {
    Exited(i32), TimedOut, OutputLimited, ResourceLimited,
    PolicyDenied, SetupFailed, TeardownFailed,
}
#[derive(Clone, Debug)]
pub struct WorldResult {
    pub disposition: WorldDisposition,
    pub stdout: Vec<u8>, pub stderr: Vec<u8>,
    pub teardown_verified: bool,
    pub image_digest: String, pub input_manifest_digest: String,
    pub host_policy_digest: String,
    pub execution_evidence: serde_json::Value,
}
#[async_trait::async_trait]
pub trait AcceptanceWorld: Send + Sync {
    async fn execute(&self, request: WorldRequest) -> crate::WorkflowResult<WorldResult>;
}
```

`WorldLimits`, root mappings, image, network and environment belong to the adapter's
host policy, **not** `WorldRequest` or model output. Persist strict versioned JSON
policy at launch with BLAKE3 binding in the observer snapshot. Reject unknown policy
fields and refuse changed policy on recovery; an explicit new evaluation may bind
a different policy but must not rewrite earlier evidence. Preserve absent-snapshot
legacy behavior. Finalization/observer configuration is never a prerequisite for
ordinary legacy admission.

### 9.0 Linux build feasibility — execute before world implementation

- [ ] Pin a Linux arm64 base image and Rust 1.96.1 toolchain, create a clean
  committed-source snapshot excluding protected dirty WIP, and record every input.
- [ ] Provision the toolchain/system dependencies in a dedicated owned container.
  Provisioning may download public packages; this is not a model-command world or
  a proof of offline acceptance execution. No host credential/config mounts.
- [ ] Build `cargo build --locked --release --bin archon` once, unchanged default
  features and release profile; record toolchain, packages, image digest, source
  tree/manifest and complete log. Do not run workspace tests or frozen checks.
- [ ] One-hour bound across provisioning/build. Limit jobs to two and container
  resources to avoid starving existing services. Verify `archon --version` on
  success. Failure/timeout stops this slice; no Linux-porting fixes or substituted
  host build without returning the exact blocker to the operator.

### 9.1 Host policy, inventories and guest paths

- [ ] **Red tests:** deserialize missing/unknown policy keys; reject a writable host
  bind, path escape, overlapping scratch destinations, replacing task/contract
  files, source symlink escape, socket/device/FIFO, and ambiguous project/repository
  collisions. Model strings resembling env assignments or mount flags remain stdin.
- [ ] Define closed configuration under `[workflow.acceptance_world]`: `enabled`
  default false; `backend`; digest-pinned image; validated local runtime endpoint;
  explicit project/repository/task root bindings; `project_repository_view` default
  `separate`; build target relative path; copied writable directory list;
  allowed input exports; toolchain/cache seeds; closed resource profile; endpoint
  broker rules default empty. Host paths are config-relative then canonicalized;
  the accepted check may select only existing `TrustedCwd` enum values.
- [ ] Root binding comes from the implementation run's host-resolved repository,
  not from where the CLI happens to be installed. Record both host and guest roots.
  Export immutable no-follow snapshots with digests and omission reasons. Strip
  host provider credentials, workflow configs, `.git` secrets, runtime sockets,
  devices and existing build artifacts. Do not hide required source/project files
  in the name of filtering: preflight reports what is absent.
- [ ] Preserve exact relative command paths. Default separate views map guest
  project and repository separately. If the operator explicitly selects
  `project_repository_view = "combined"`, assemble a **read-only** project-cwd
  view from the declared repository source plus declared project inputs. A
  manifest in repository and no manifest in project must resolve in this view;
  `--target .` must still see project inputs. Refuse colliding nonidentical files
  without an explicit config ownership rule. Never auto-select combined mode by
  detecting `cargo` in the model's text. Capture the mapping/digest in evidence.
- [ ] Tasks and frozen chain stay immutable in every guest view. Writable data
  destinations cannot cover root, source, Cargo manifests/lockfile, tasks, pins or
  toolchain. Layout construction happens entirely in new run-owned export/staging
  storage; no links, files or directories are inserted into live project roots.

### 9.2 Real build and mutable-data scratch

- [ ] **Red positive fixture:** a small generic Rust project checks
  `cargo build --release && ./target/release/probe --target .`, reads a seeded
  project data file and mutates that data. Assert an actual guest build ran, the
  invoked executable came from that build, data writes persist within that
  invocation, and original source/project hashes remain unchanged. No prebuilt
  host executable or fake cargo stand-in satisfies this test.
- [ ] Host creates a fresh quota-bounded scratch filesystem per check. Set only
  fixed, recorded guest values, starting from an empty inherited environment:

```text
PATH=<pinned toolchain executable path>
HOME=/scratch/home
CARGO_HOME=/scratch/cargo-home
CARGO_TARGET_DIR=/scratch/build/target
RUSTUP_HOME=<pinned guest toolchain home>
TMPDIR=/scratch/tmp
CARGO_NET_OFFLINE=true
```

  The toolchain image and dependency seeds are provisioned by the operator;
  credentials/config from host HOME are never copied. Cargo registry/cache is
  copied to scratch from a verified credential-free seed. Network is not opened
  to package registries to rescue a missing offline dependency. Root fingerprints,
  target triple, compiler/nextest versions (if provisioned), flags and cache seed
  digests are recorded. Only config-owned nonsecret service environment bindings
  may be added. The model cannot widen these bindings.
- [ ] In the guest assembly, the configured relative `target` destination refers
  to **the same scratch target directory** as `CARGO_TARGET_DIR`; arrange it using
  a host-generated mountpoint/link in the exported layout, never by editing a live
  source directory. Both `./target/release/...` and `$CARGO_TARGET_DIR/release/...`
  must select the newly built guest output. Merely changing the environment while
  leaving `./target` pointing at the read-only old tree is a failing implementation.
- [ ] For each configured mutable project directory, copy its snapshot into a
  distinct scratch directory, preserve its declared relative guest location and
  make **only that copy** writable. Use the same mapping from both cwd views when
  appropriate, but a separate private copy for each check. Record original/copy
  digests and mutations; do not copy changes back to the live project.
- [ ] Guest filesystem layering consists of immutable source/project baseline
  plus explicitly declared scratch submounts. No general writable source overlay,
  no writable task root, and no model-selected submount. Missing required directory
  has an explicit config policy (must exist vs initially empty); do not fabricate
  datasets to turn a check green.
- [ ] Set host resource budgets from the operator's approved build profile, not
  the earlier arbitrary 1-GiB/short-command limits. Generic fixture passes within
  its small test profile; real-workload readiness must establish enough time,
  RAM, PIDs and scratch for a native guest build. Missing Linux/platform toolchain
  support is `SetupFailed`, not claimed fixed by Docker existing. Never substitute
  a host build for unsupported guest compilation.

### 9.3 Deny-all networking with complete denial observation — first slice

- [ ] No endpoint allow rules ship in this slice. Place each world on a dedicated
  internal network with one owned logging blackhole peer; no bridge to services,
  host gateway, public DNS or Internet. Configure routing to the collector for
  observable outbound traffic, and validate actual effective routes/firewall.
- [ ] Do not equate received blackhole packets with all attempted connections.
  Guest loopback, locally rejected sockets, IPv6, UDP/DNS and hardcoded addresses
  may never produce a packet at that peer. Add a trusted, non-bypassable network
  attempt observer at the guest boundary (syscall/network-namespace enforcement)
  covering those cases. Instrumentation/collector health is attested before and
  after the command. Missing coverage, dropped records or collector failure is
  operational failure, never an unobserved pass. Do not use LD_PRELOAD or proxy
  variables as complete enforcement/observation.
- [ ] Record every attempted network destination/protocol and denied outcome;
  exclude only host-owned runner infrastructure operations identified independently
  of model output. Guest policy cannot suppress logs or widen network authority.
  `PolicyDenied` wins over command exit zero and grepped unavailable messages.
- [ ] Red cases: a generic command catches refused external/loopback/IPv6/DNS/UDP
  attempts, prints `unavailable`, exits zero. Every case returns `PolicyDenied`.
  No-network positive command passes. Collector failure/dropped logs fail closed.
- [ ] Verify no host or Internet connection succeeds, no inherited service access,
  no attached Docker/control socket, and the blackhole cannot forward traffic.
  Teardown removes the world, collector and dedicated network only.
- [ ] Brokered read/probe endpoints are a follow-on, separately approved task.
  Keep the detailed broker policy in the amendment as deferred design, not a
  first-slice requirement. No protected service is contacted or reconfigured.

### 9.3C Run-scoped trusted build cache

- [ ] Build the same immutable source/toolchain/dependency inputs once under a
  trusted host-owned build invocation; seal the resulting target/cache seed and
  record its content manifest. This preparatory build has no model command input.
- [ ] Key by source and lockfile bytes, target triple/compiler/toolchain/image,
  build profile/features/flags, dependency/native-library seeds, guest source
  paths and metadata used by Cargo freshness. No cross-run reuse in this slice.
- [ ] Before each check, verify the sealed seed and clone it to private scratch
  at the exact guest target path. Checks may modify their private copies; **never
  promote check output back into the seed**. All checks get the same pre-check
  immutable seed, not the previous check's artifacts.
- [ ] Red tests poison a binary/fingerprint in check A, then prove check B starts
  from trusted seed bytes and source changes invalidate cache identity. Preserve
  Cargo-relevant path/mtime identity or let Cargo rebuild; do not fake freshness.
- [ ] Positive control executes Cargo in each check and confirms it reuses valid
  artifacts without rebuilding unchanged workspace inputs; the actual resulting
  binary is invoked. Measure reuse and setup/build time, not just hash equality.
  Mutable project data remains per-check and never cached across checks.

### 9.4 Supervision, evidence and minimum acceptance-slice lifecycle

- [ ] Proposed backend remains digest-pinned local Docker Desktop Linux image,
  read-only container root, no privileges/capabilities, no host PID/device/socket
  mounts, quota-bounded scratch, fixed entrypoint and stdin-delivered command.
  Inspect actual create configuration before start; reject image-declared volumes,
  unexpected mounts, env, network or privileged configuration.
- [ ] Owned guardian process attaches bounded stdin/stdout/stderr without TTY and
  holds a parent-liveness pipe. EOF/timeout/overflow/crash tears down container,
  broker and bridge, then verifies absence; stdout/stderr are drained concurrently.
  Startup rejects a second observer while its OS world-execution lock is held.
  This narrow lock/guardian ships with Task 9; it does not need the general
  executable-snapshot machinery of Task 5 or full claim recovery of Task 8.
- [ ] Container-side watchdog bounds lifetime even if the host-side Docker client
  disappears. Parent plus guardian SIGKILL must not leave indefinite workloads:
  persist owned world identity and deadline before start, reconcile orphan IDs
  before a later observer launch, and refuse to claim teardown if runtime is
  unreachable. No automatic repeat of an interrupted network-bearing check.
- [ ] Evidence includes command digest, selected cwd/layout, policy/input/image
  digests, toolchain, build target path, copied data inventories, exact approved
  endpoint rules and denial log, actual container settings, resource observations,
  output hashes/status, and whole-world teardown receipt. Never record secrets.
- [ ] Positive control must run a build + built binary + scratch mutation, not just
  `/bin/true`. Hostile tests attempt writes to immutable input/live sentinels,
  credential reads, mount/symlink escape, unapproved endpoint access, fork/detach,
  floods and deadline overruns. Verify host bytes unchanged and owned components
  gone. Model attempts to reset env/proxy settings must not bypass mount/network
  enforcement; config values are not a security boundary by themselves.
- [ ] Distinguish `Exited(nonzero)` from infrastructure/policy denial. Read-only
  artifact checks may fail normally because an artifact is absent; do not call
  every nonzero “containment”. Detected policy denial or setup/resource failure is
  operational even when the shell exits zero or greps an “unavailable” message.
- [ ] **Sabotage actual wiring:** remove scratch-target mapping, data-copy bind,
  egress restriction, empty-env reset or guardian EOF branch separately. Require
  the corresponding build/mutation/security/teardown test to fail. Do not leave
  hostile variants enabled or touch unrelated services.
- [ ] Commit and run the approved generic suite. Frozen real commands are **read
  as data only** for a readiness matrix until the operator authorizes execution;
  no PRD-specific configuration is hardcoded into engine or generic fixtures.

## Task 10 — Route all command-bearing observer checks through the world

**Files**
- Create `src/command/workflow_run_end_checks.rs`, `workflow_run_end_residuals.rs`, `workflow_run_end_world_tests.rs`.
- Modify `workflow_run_end_observer.rs`, `workflow_live_v2_finalizer.rs`, and existing observer test implementations.
- Reuse `validate_residual_gaps`, `collect_declarative_floor_facts`, `evaluate_declarative_floor` and the existing advanced deliverable kernels; locate their current exports before moving code. Do not create another parser/evaluator.

**Execution order: SECOND.**

**Consumes:** Task 9 `AcceptanceWorld` and its narrow execution lock/guardian, existing immutable terminal snapshot and single-owner R2a finalizer. Task 8 later upgrades crash recovery; it is not a prerequisite.
**Produces:** complete per-criterion evaluation plus coverage records with `ObserveOnly` authority.

- [ ] Convert `WorkflowRunEndObserver::observe` to an async trait method where needed; await it after terminal commit rather than nesting a Tokio runtime or blocking its event loop. Keep context and outcomes unchanged unless an optional versioned count is required.
- [ ] Table-driven red test routing:

```text
Command            → exactly one WorldRequest(kind=Command)
Floor, no command  → existing pure floor kernel; zero world requests
Floor, command     → existing floor prerequisites AND world NestedVerifier
Residual record    → only after valid allowed failed criterion; ResidualFailClosed
LegacyAbsent       → zero probes, zero records, zero world requests
```

Use a recording port to make the production observer call fail if any path reaches a host shell; backend containment itself is proven by Task 9, not that mock.

- [ ] For each world result: `Exited(0)` + verified teardown + no recorded execution-policy denial = pass; normal `Exited(nonzero)` = failed criterion; `PolicyDenied`, timeout, output/resource limits, setup failure or unverified teardown = operational observer failure/deferral. None changes terminal status. Never count an unexecuted/operational check as evaluated-and-passed.
- [ ] Residual coverage algorithm:

```text
validate file structure and allowed IDs using existing kernel
reject unknown/supplementary/passing/non-permitted/duplicate criterion coverage
for each permitted failed criterion:
  exactly one authorized record + isolated fail_closed_check pass → covered shadow
  no record / failed check → uncovered shadow
  operational check failure → operational record, not covered
```

- [ ] Test passing criterion with stale gap, duplicate IDs, forbidden phrase, supplementary coverage, absent gaps, malformed gaps, failed and passing fail-closed checks. Test global enforce leaves terminal bytes unchanged. Frozen identity mutation between launch and observer remains operational.
- [ ] **Sabotage each routing call separately:** Command, nested verifier and residual must each go red when its world dispatch is removed. Leaving the trait/helper in place is not adequate evidence.
- [ ] Commit; run observer/finalizer/residual/legacy suites, then Task-9 generic world tests. No live model invocation needed for these deterministic tests.

## Task 12A — Acceptance-slice verification and stop

**Execution order: THIRD**, immediately after 9–10. This verifies those features
against existing R2a behavior, not unfinished Tasks 1–8 or 11.

**Files:** new generic fixtures and `tests/r2b_acceptance_world_live.rs` from Task 9;
`tests/r2b_acceptance_world_readiness.rs` for strict policy/readiness checking;
evidence under an operator-selected scratch directory named by the committed build.
No test fixture contains protected-project vocabulary. Update the ledger by explicit
path only after observed results.

- [ ] Gate with the amended spec/plan approval, inline one-coordinator execution,
  clean-source checkout and committed revision before every build. Preserve
  protected WIP untouched; no retrospective source manifest.
- [ ] Compile root binaries/tests; run observer/finalizer/legacy/dispatch-wrapper
  suites and generic world tests, then the approved real Docker build/data/denial-observation
  probes. The positive build must exercise the actual guest toolchain and built
  binary. Test separate and explicit combined project/repository layouts.
- [ ] Make a per-check readiness matrix using the frozen bytes as data: exact cwd,
  repository manifest availability, offline dependency/toolchain readiness, target
  alias, writable copied directories, guest endpoint routes, required inputs,
  resource profile and platform compatibility. No acceptance check is marked
  runnable because its kind is Command or because the judge accepted it.
- [ ] If separately approved, execute unchanged frozen checks **only inside the
  world** and report executed/pass/failed-criterion/operational separately. Before
  implementation, absent functionality may legitimately fail. Do not require a
  green implementation outcome to release a correct isolated-check mechanism,
  and do not label a universal setup failure as evidence of working acceptance.
- [ ] Capture actual mount/env/network policy, build/source/data identity, endpoint
  allow/deny evidence, output/status and teardown. Repeat policy-denial checks
  with shell attempts to catch errors and return zero. Confirm ObserveOnly under
  global enforce, LegacyAbsent silence and zero unexpected host mutations.
- [ ] Independent review must inspect every actual world-routing wrapper, scratch
  mapping, egress enforcement and cleanup path. Execution is inline; a separate
  reviewer can review the diff/evidence without spawning implementation agents.
- [ ] Build clean committed release and record compiler/target/flags/source/image
  identities and hashes. Deployment and external implementation require separate
  approval. **Stop for acceptance-slice review; do not claim all R2b complete.**

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

## Task 5 — Run-owned executable image and independent guardian

**Files**
- Create `src/command/workflow_executable_snapshot.rs`, `workflow_host_guardian.rs`, `workflow_executable_snapshot_tests.rs`, `workflow_host_guardian_tests.rs`.
- Modify `workflow_decompose.rs`, `workflow_decompose_resume.rs`, `workflow_host_command_catalog.rs`, `workflow_host_command_supervisor.rs` and CLI internal dispatch in `src/cli_args/strategy_actions_workflow.rs` / `workflow_decompose_cli.rs`.

**Consumes:** writer fence, Task-1 durable writes, host-resolved `current_exe`.
**Produces:** `ExecutableSnapshotV1 { relative_path, sha256, byte_len, binary_revision, catalog_digest, script_digest }`; a same-image startup handshake before child reads candidate stdin.

- [ ] **Red tests:** copy executable A into a temporary installation path, launch a run, atomically replace installation with executable B, and invoke another host stage. It must still execute snapshot A; corrupted snapshot bytes must refuse before the candidate reaches any child. Do not overwrite real installed binaries for this test.
- [ ] **Implement:** open source image once, hash/copy from that fd into a run-owned create-new file, fsync, set executable non-writable mode, record manifest. Retain root/image identity. Rehash before each spawn and require an internal handshake containing expected snapshot hash/revision/catalog plus a host nonce. Candidate bytes are withheld until handshake succeeds. A process reports its own **loaded image identity**, not merely re-reading an arbitrary path supplied in argv; on macOS bind the executable vnode/image identity and validate the supported replacement threat model in a platform test.
- [ ] Cross-version outer launcher must not silently execute old scripts using new code. A mismatch names the verified snapshot-specific recovery command. Automatic delegation, if offered, is explicit host policy and never loads a model-selected executable.
- [ ] **Guardian:** dedicated trusted child owns command spawning and listens on an inherited parent-liveness pipe. EOF triggers teardown/reap independent of the parent destructor. Pipe fds are close-on-exec everywhere except the intended guardian endpoint; command descendants cannot keep the liveness pipe open. Return success only after bounded output/drain/reap and guardian acknowledgement.
- [ ] Kill the parent with SIGKILL while command is running; assert guardian cleans its managed process tree. Also test cancelled/paused generation, timeout, output flood and pipe-held descendant. For trusted host capabilities detaching remains forbidden. Do **not** describe PGID probing as detecting `setsid` escape; acceptance-world teardown in Task 9 covers arbitrary command descendants.
- [ ] **Sabotage:** resolve installed path at the live call site rather than snapshot; A/B replacement test fails. Drop the guardian's EOF branch; parent-death test fails.
- [ ] Commit; run supervisor, resume identity, lifecycle shutdown and new image tests. A failed platform enforcement probe blocks enabling live-replacement support rather than weakening the claim.

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
- [ ] Re-run acceptance-slice generic regression/security tests after integrating
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
| Isolated Command, nested verifier and residual check | First slice 9.0 → 9–10 → 12A; real guest build/data/denial controls and routing sabotage |
| Explicit predecessor adoption | 11; opt-in/no-mutation/exact-byte/legacy tests |
| Snapshots, handshake, live replacement | 5; A/B image swap and parent-death tests |
| Canonical writer leases, cross-run CAS, no-follow | 1–4; separate-process writer races and ancestor swap |
| Composite journal and committed-result adoption | 1, 4; every fsync/rename/receipt crash cut, judge count stays one |
| Crash-active author ledger/active-time recovery | 6; pause clock and prepared-result/ambiguous-request cases |
| Generation-CAS and event allocation/append | 2; competing writers, stable event IDs and torn append recovery |
| Progress outbox replay | 7; event-before-log/log-before-ack crashes |
| Observer claims and finalization-only recovery | 8; competing finalizers, every terminal/observer boundary |
| ObserveOnly and legacy behavior | First 10/12A; later 8/11/12B; unchanged terminal state under enforce and no legacy probing |

- [ ] Approve the separate spec amendment: immutable host/input baseline, declared scratch build/data submounts, sanitized host env, default-deny brokered endpoint policy. Original spec remains authority until approval.
- [ ] Approve plan execution separately; current task produced planning documents only.
- [ ] Preserve historical R2a provenance exception as scoped; no future relaxation.
- [ ] All helper interfaces above have a named owning task and production integration point.
- [ ] New Rust implementation files are split before 500 lines; tests use the nearest existing fixtures.
- [ ] Independent review and release evidence remain gates, not completed checkboxes.


## Approval update — first slice only

Operator approved amendment/reorder with three conditions: 9.0 pinned Linux build
first (stop on failure), deny-all network with complete attempt observation before
any broker, and a trusted run-scoped build-cache seed cloned per check. Execute
inline, one coordinator. Tasks 1–8/11 and endpoint allowance remain separately
gated. The approximate five-executable/six-provider-denied split is an expectation,
not a hardcoded classification: report actual per-check outcomes and network attempts.
