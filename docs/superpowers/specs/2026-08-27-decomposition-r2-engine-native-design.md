# Decomposition R2 Engine-Native Design

**Date:** 2026-08-27
**Scope:** R2 only — add a bounded persisted `hostCommand()` capability, a fixed v3 decomposition meta-script, durable TUI/CLI lifecycle visibility, provider-neutral authoring, an observe-only run-end acceptance observer, and two live proof packages
**Inputs:** `/Volumes/Externalwork/DECOMPOSITION-FIX-PLAN.md`, `/Volumes/Externalwork/HANDOVER-CODEX-V3-DECOMP.md`, the amended R2 boundary, the independently cleared R1 commits `716469d04` and `c0508640d`, and review findings C1–C5, D1–D3, E1–E2, and F1–F2

## Objective

R2 makes decomposition a first-class, persisted v3 workflow rather than an agent relay or a second host-side workflow engine. A fixed, hand-authored meta-script drives provider-neutral author calls and deterministic host stages through a generic but tightly bounded `hostCommand()` capability. Every material transition is durable, inspectable through ordinary workflow status, resumable after interruption, and mirrored to the TUI and `.decompose.log`.

R2 ends with two distinct proofs:

1. A minimal synthetic scratch project is decomposed and then run through a complete normal v3 implementation lifecycle, including an observe-only run-end acceptance evaluation.
2. `PRD-TRADING-DATA-LAKE-AHDM-001` is decomposed end to end under observing gates. No implementation run is launched for that task set.

R2 does not promote a gate, start R3, or modify protected trading implementation code.

## Authority and inherited invariants

The corrected v4 rollout controls. The v3 handover supplies its operational interpretation. Older correctness documents remain authoritative only for schemas, validation kernels, freeze order, and lint mechanics that do not conflict with R1 or the amended R2 boundary.

The following R1 invariants remain unchanged:

- `workflow.gate_mode` defaults to `observe`.
- `off` performs no gate evaluation and emits `gate_mode=off — nothing was evaluated; this is not a pass` only for an explicitly invoked analysis.
- Observe preserves ordinary reports, emits verbatim `[shadow]` findings, and records one JSONL record per finding without blocking on policy.
- Operational and integrity failures block in every mode.
- Normal legacy workflow admission never consults acceptance contracts, locks, skeletons, pins, or freeze provenance.
- A wholly absent freeze chain is valid legacy input. A partially present, malformed, mismatched, or stale chain is operationally broken.
- Nothing in R2 connects decomposition freezes to start-of-run admission for pre-existing task sets.
- Generic source, comments, fixtures, and scripts contain no protected-project task IDs, domain vocabulary, corpus paths, or trading terminology.
- No push or CI trigger occurs.
- Nothing under `crates/archon-trading` is edited, staged, or committed.
- OpenBB on `6900`, LiteLLM on `1234`, TradingView MCP, and the cognitive daemon are never killed or reconfigured.

## R2 completion boundary

### Trading PRD

R2 takes the trading PRD through:

```text
acceptance authoring
→ acceptance freeze
→ skeleton authoring
→ skeleton freeze
→ every body through per-file lint
→ final task-set lint and requirements trace
→ durable terminal decomposition status
→ status/resume proof
→ shadow-log review
```

Then it stops. No implementation workflow is launched over the generated trading task set. Launching that implementation run remains Steven's decision after R2 evidence review.

### Synthetic project

The run-to-terminal proof uses a clean scratch Git project with generic vocabulary, two or three trivial tasks, and scratch-only target files. It proves the normal v3 implementation engine and the run-end acceptance observer without touching protected paths.

## Delivery phasing — happy path before hardening

R2 is delivered as two internal increments under this approved architecture.

### R2a — critical spine and proof gate

R2a contains only the shortest path required by both proof packages:

- fixed `FixedDecompositionV1` run kind, launcher, and embedded raw-`w` meta-script;
- persisted command-capability catalog, host-owned token rebinding, trusted provider route, and numeric I/O/time bounds;
- audited non-detaching Archon subcommands through normal persisted `HostCommand` records;
- child staging plus parent-only two-phase publication, exact final-byte receipts, and mutation audits;
- acceptance/skeleton/body/set-gate phases and run-metadata-only shadow outcomes;
- bounded author attempts, including pause/cancel without attempt advancement;
- existing v3 call records/checkpoints extended with fixed phase/body state, one active executor, basic durable events, TUI/CLI streaming, `.decompose.log`, status, pause/cancel/resume, and state-before-terminal-event ordering;
- launch-time observer eligibility plus observe-only evaluation of command-free declarative floors;
- the synthetic full-lifecycle proof and trading decomposition-only proof.

R2a targets fresh/empty task roots and one active executor per proof. It does not promise crash-perfect recovery or concurrent-run correctness. A process crash may require explicit resume from the last durably completed phase; incomplete staged output is discarded or treated operationally, never silently accepted.

### R2b — post-proof hardening

R2b begins only after both R2a proof packages pass and their evidence is reviewed. It preserves the R2a interfaces and adds hardening without redesigning the proven phase spine:

- mandatory ephemeral isolated acceptance world for model-authored `AcceptanceCheck::Command`, nested floor `typed_verifier_command`, and residual-gap `fail_closed_check`;
- explicit non-mutating `AdoptedPredecessorReceipt` import for pre-existing portable chains;
- run-owned executable snapshots, per-spawn same-image handshakes, and live installed-binary replacement defense;
- canonical task-root writer leases, cross-run CAS, retained-directory-handle/no-follow publication, and platform-specific OS confinement if a real primitive is selected;
- composite crash journals, committed-publication adoption, and exact crash recovery across every multi-file boundary;
- crash-active author dispatch ledgers and active-time heartbeat recovery;
- OS-locked generation-CAS state/checkpoint transactions and cross-process event sequence+append transactions;
- progress outbox replay, exclusive stale-recoverable observer claims, and finalization-only startup recovery.

Until R2b exists:

- command-bearing acceptance checks and residual-gap fail-closed checks record post-terminal observer-operational deferral and never execute on the host;
- pre-existing chains without R2a run-owned receipts are refused/re-frozen, never silently adopted;
- a second process/run targeting the same task root is outside supported R2a operation and is refused by launcher policy rather than coordinated;
- a binary deployment is forbidden while an R2a run is active; resume requires the starting binary revision plus script/catalog digests;
- crash recovery never adopts a partially published nondeterministic freeze as accepted.

### Honest macOS write-safety claim

R2a does **not** claim Seatbelt or arbitrary OS-enforced per-path confinement. Its implemented mechanism is:

- only audited, trusted, non-detaching Archon subcommands are eligible;
- model bytes enter only bounded stdin;
- argv/cwd/environment/write-set declarations are host-owned and digested;
- children write only to a run staging root by command contract;
- the parent enumerates staged outputs and requires an exact declared-path match;
- pre/post sentinel and repository mutation audits detect any unexpected host write;
- the parent alone commits staged outputs to live task paths and records exact final-byte receipts.

These controls detect/refuse undeclared effects from trusted built-ins; they are not a security sandbox for an arbitrary malicious executable. If R2b adds OS confinement, the selected macOS primitive and its enforcement tests must be named explicitly.

### Preserved R2b predecessor-adoption design

R2b's `AdoptedPredecessorReceipt` is non-mutating and available only through explicit launcher policy. It binds:

- canonical task-root identity;
- exact contract/skeleton/lock/pin digests;
- full integrity, provenance, predecessor-policy, and current-binary validation results;
- inherited predecessor shadow record IDs/digests;
- adopter run/binary/script/catalog identities;
- adoption timestamp and stable event ID.

Reuse requires the receipt plus exact current digest equality and the ordinary host kernels. Missing/mismatched artifacts invalidate adoption; internal consistency alone never authorizes a skip.

### Invariants unchanged across R2a and R2b

- Models never author executables, argv, cwd, environment, catalogs, locks, pins, receipts, or script structure.
- Raw model-authored command text never executes on the host.
- Host stages remain first-class persisted calls visible to status/resume.
- Existing gate commands remain the only artifact validators.
- Policy findings do not block in observe; operational/integrity failures always stop the affected phase.
- Legacy admission stays freeze-unaware.
- Run-end authority remains `ObserveOnly` until R4 promotion.
- Protected trading implementation remains untouched and no trading implementation run is launched.

## Architecture decision

### Selected: persisted command-capability catalog

The fixed decomposition script calls symbolic host capabilities. Before execution, the host resolves and persists a digested command catalog. The script can supply bounded stdin candidate bytes, but it cannot author an executable, argv element, cwd, environment name, timeout, output limit, or reuse key.

```javascript
const outcome = await w.hostCommand("freeze-acceptance", {
  stdin: authored.content
})
```

The fixed script contains the literal `freeze-acceptance` identifier. Model output can reach only `stdin`.

### Rejected: trusted-script raw argv

Allowing an embedded script to construct `{ executable, argv, cwd }` makes script arguments and future script mistakes process-construction authority. Script-digest trust alone is too broad and makes command review non-local.

### Rejected: bespoke Rust decomposition orchestrator

A TUI-local or binary-local orchestrator would need its own retries, checkpoints, resume rules, event stream, cancellation, and status interpretation. That recreates the failed second-engine architecture and violates the v3-only target.

## System boundary

```text
TUI / CLI
  │ workflow decompose --prd <path> --tasks <dir>
  ▼
FixedV3RunLauncher
  ├─ validates project, PRD, task-root, and protected-path boundaries
  ├─ persists the run before background execution
  ├─ persists run kind, fixed source, arguments, command catalog, and digests
  ├─ creates the run-owned progress reporter and execution lease
  └─ returns run_id before execution begins
        │
        ▼
normal v3 runtime
  ├─ w.agent(... resultMode: "rawOutcome")
  ├─ w.hostCommand(commandId, bounded stdin)
  ├─ ordinary call records, events, checkpoints, and control polling
  └─ ordinary pause, cancel, status, and resume
        │
        ▼
binary-owned HostCommandExecutor
  ├─ catalog lookup and token rebinding
  ├─ admission, cwd, environment, and protected-path policy
  ├─ journaled candidate transaction
  ├─ bounded process execution and process-tree cancellation
  └─ typed persisted result
```

The fixed script uses the raw `w` API. `w.hostCommand` is added to the authoritative object assembled by `script_source`; adding only a convenience prelude/global is insufficient because valid v3 scripts may use no prelude helper at all.

`HostCommand` remains in the ordinary persisted host-call path. It is not diverted like `runTool`, because that would remove call records, checkpoints, status visibility, and resume reuse.

## Persisted run identity

Generated-run metadata replaces the overloaded script-lifecycle boolean with a backward-compatible run-kind enum:

```text
AuthoredTaskWorkflow
LegacyDecomposed
FixedDecompositionV1
FixedOrSavedScript
```

An R2a fixed decomposition run persists run kind/template version, exact script and catalog digests, starting binary revision, canonical args/project/PRD/task root, `.decompose.log`, phase/body outcomes, attempt counters, and normal v3 call/checkpoint state.

R2a forbids binary deployment while a run is active. Resume requires the invoking binary revision plus embedded script/catalog digests to equal persisted values; mismatch refuses with the named binary remedy. R2b adds executable snapshots, image hashes/handshakes, and live replacement safety.

## `hostCommand()` contract

### JavaScript API

```javascript
const result = await w.hostCommand("task-set-lint", { stdin: null })

// result
{
  exitCode,
  stdout,
  stderr,
  stdoutBytes,
  stderrBytes,
  timedOut,
  interrupted,
  stdoutTruncated,
  stderrTruncated,
  gateEnvelope,
  publicationReceipt
}
```

The script does not supply `reuseKey`, executable, argv, cwd, environment, timeout, limits, destination paths, or write sets.

A non-zero exit is process data available to script control flow. Malformed requests, undeclared capabilities, token violations, spawn failures, policy denial, timeout-enforcement failure, process-reap failure, output overflow, transaction failures, and missing/mismatched receipts are host operational errors.

### Command capability

```text
CommandCapability
  id
  program_identity
  argv_template[]
  token_definitions[]
  cwd_policy
  stdin_delivery
  environment_profile
  timeout_secs
  max_stdout_bytes
  max_stderr_bytes
  declared_write_set[]
  containment_policy
  gate_envelope_policy
  postcondition
```

Every `declared_write_set` entry identifies an exact staged output or tightly bounded append record derived from host-owned tokens. Built-in capabilities declare contract/skeleton bytes, lock, pin, body, envelope, provisional receipt, and shadow records. `.decompose.log`, run events, and state remain parent-owned.

The catalog write set is digested and reviewable. In R2a audited children target a run staging root. The parent enumerates the staged tree, refuses undeclared outputs, compares protected/repository mutation sentinels, and alone commits exact staged paths. Any mismatch is operational. R2a explicitly does not claim OS enforcement inside the child; R2b owns stronger confinement/TOCTOU controls.

Only trusted host launchers/templates may supply a catalog. Model-authored and ordinary authored scripts cannot declare capabilities. Calling an undeclared `commandId` fails before spawn.

### Token authority and validation

Every dynamic argv token declares both its host source and validator. Script-provided token values have no authority.

- `ProjectRoot`, `PrdPath`, and `TaskRoot` equal launcher-owned canonical paths.
- Paths remain within the declared root, reject traversal, and reject symlinks/symlink escape.
- Existing/absent paths are canonicalized through trusted parents and symlink descent is rejected.
- `FrozenTaskId` matches canonical `TASK-<AREA>-<NNN>` grammar.
- `FrozenTaskFile` is a direct `TASK-*.md` child of the canonical task root.
- Task ID and filename equal the tuple obtained by the host re-reading the serde-validated frozen skeleton on disk.
- Immediately before parent commit, expected old-content digests and repository mutation sentinels are rechecked.
- Any mismatch is operational before candidate publication or spawn.

The script may iterate model-returned skeleton data for scheduling, but host rebinding prevents those bytes from becoming argv authority.

### Host-computed call identity and produced-output binding

The executor computes:

```text
BLAKE3(
  "host-command-v1"
  || command_id
  || catalog_digest
  || starting_binary_revision
  || canonical_resolved_token_map
  || exact_stdin_bytes
)
```

The script cannot override it.

After the parent supervisor commits a successful prepared publication, the accepted call record persists a content-addressed `PublicationReceipt` containing:

- command identity and invocation ID;
- exact produced contract/body, lock, pin, and envelope IDs as applicable;
- BLAKE3 digest of every produced/replaced immutable file;
- stable shadow-record IDs plus canonical per-record byte digests and locked membership proof, never the mutable whole JSONL digest;
- expected prior digest/CAS value for every replacement;
- terminal process/output metadata established by the parent;
- publication commit sequence.

Reuse requires:

1. matching persisted/current binary, script, and catalog identities;
2. an accepted call record with the host-computed identity;
3. zero exit, no timeout, no interruption, no truncation, and no infrastructure failure;
4. a committed receipt whose complete produced-output digest set exactly matches the current filesystem;
5. the existing host integrity kernels still accepting that exact chain/body.

It is not enough for the current filesystem to form some internally valid freeze. It must be the exact version produced by this accepted call. A non-zero, timed-out, interrupted, truncated, or operationally failed call is never reusable.

### Input and output limits

Initial R2 catalog limits are:

| Capability | stdin | stdout | stderr | timeout |
|---|---:|---:|---:|---:|
| acceptance candidate + freeze | 2 MiB | 2 MiB | 2 MiB | 1,500s |
| skeleton candidate + freeze | 2 MiB | 2 MiB | 2 MiB | 1,500s |
| task body candidate + lint | 1 MiB | 1 MiB | 1 MiB | 300s |
| task-set lint | none | 4 MiB | 4 MiB | 300s |
| requirements trace | none | 4 MiB | 4 MiB | 300s |

Stdin overflow rejects before any write. Stdout and stderr are drained concurrently while the process runs. Crossing either output limit terminates the supervised process and records `OutputLimitExceeded`. A bounded preview may be retained as diagnostics, but truncated output is never accepted, checkpointed, or published.

UTF-8 conversion is explicit and lossy only for bounded diagnostic display; exact byte counts remain available. Candidate stdin must satisfy the target command's UTF-8 contract before the command may publish it.

### Environment policy

Every child begins with `env_clear()`.

- Freeze capabilities receive a named provider environment profile.
- The catalog persists variable names and resolution sources, never values.
- Values are resolved from Archon configuration/provider resolution, not the parent shell.
- Lint, trace, and body-landing capabilities receive no environment variables.
- No wildcard inheritance is allowed.
- `HOME`, `PATH`, API keys, and endpoints exist only if that exact capability declares them.
- Environment values never enter catalog digests, events, call records, or `.decompose.log`.

The same configured-only endpoint policy applies to decomposition `w.agent` calls and freeze judge calls. For R2, `ConfiguredOnly` means a `TrustedProviderRouteSnapshot` resolved exclusively from an explicit CLI/operator selection or user-level credential/provider configuration outside the repository. Repository/project-local `.archon` and project `config.toml` layers may select semantic tiers and non-secret behavior, but cannot supply or override endpoint URLs, credential profiles, keys, or proxy trust. Using a repository-local route requires a separate explicit operator approval that persists the exact endpoint/profile source before any repository content is sent.

Fixed-decomposition client construction ignores ambient `ANTHROPIC_BASE_URL` and unapproved repository endpoint overrides. The run persists endpoint origin/profile provenance and a non-secret endpoint digest, never credential values.

Regression tests seed hostile `ANTHROPIC_BASE_URL`, a repository-local hostile endpoint, and an unrelated sentinel, then prove:

1. author and judge requests use the trusted CLI/user route, not ambient or repository input;
2. an unapproved repository endpoint fails before any model request;
3. the freeze child receives exactly the declared variable names;
4. the lint child receives no environment;
5. no child or author routing path receives the unrelated sentinel.

### Permission and process policy

`hostCommand` reuses configured execution-world admission, filesystem boundary, activity sink, repeat policy, and non-interactive permission semantics. It does not silently inherit Bash permissions. `NeedsPermission` in unattended operation is denied before spawn.

R2 catalog commands are audited direct Archon subcommands. They must not interpret model bytes as shell/program text, daemonize, call `setsid`, or intentionally detach descendants. A capability that needs those behaviors is ineligible for direct `hostCommand` execution.

The parent starts a run-owned command supervisor with a liveness pipe. The supervisor:

1. starts the trusted child in a dedicated process group;
2. drains stdout/stderr concurrently;
3. races completion against timeout, persisted pause/cancel, and parent-pipe closure;
4. on timeout/control/parent death, terminates the process group and known descendants;
5. awaits reaping and pipe closure;
6. audits that no known descendant remains;
7. persists interruption/failure as non-reusable;
8. only then returns or releases the lease.

macOS process groups cannot contain a deliberately hostile `setsid(2)` escape. R2 does not claim otherwise. Safety instead depends on the load-bearing invariant that direct catalog commands are trusted, non-detaching executables with no model-authored process controls. Tests prove cleanup for every child shape the declared commands can create. If a declared command cannot meet that bounded non-detaching contract on the target platform, R2 stops rather than weakening the guarantee or invoking it directly.

Dropping a Rust future is never treated as process cancellation.

## Candidate publication transaction

The script treats candidate content as opaque bytes. It never parses acceptance JSON, skeleton JSON, or task frontmatter.

### Freeze candidates: process stdin, two-phase composite publication

Acceptance and skeleton candidates are not overlaid onto live artifact paths before validation. Their capabilities deliver exact bounded candidate bytes on process stdin to the existing freeze command. The command reads them, runs its existing serde/semantic/judge gate, and stages the complete output set through the shared durable transaction API.

The child may only create a `PreparedPublication`: fsynced temporary contract/skeleton, lock, pin, envelope, stable shadow records, expected prior digests, and a provisional manifest. It cannot rename live targets or mint a committed receipt.

Only after the parent observes zero exit, bounded closed stdout/stderr, no timeout/interruption, child reaping, a valid envelope, and complete provisional manifest does it commit. R2a rechecks expected prior digests and mutation sentinels, publishes staged outputs, then records exact final-byte `PublicationReceipt` and accepted call record. A second executor/task-root run is refused by R2a launcher policy. R2b adds cross-process leases/CAS and crash-perfect commit recovery.

The acceptance transaction covers final judged contract bytes, acceptance lock, host pin, gate envelope, and stable shadow records. The skeleton transaction covers skeleton, skeleton lock, updated host pin, gate envelope, and stable shadow records. Every target, prior digest, temporary path, backup path, and final digest is registered before staging.

Publication uses sibling staging/backup/rename and verifies exact bytes after commit. R2a treats any interrupted/incomplete commit as operational on resume and never adopts it. R2b adds retained-directory-handle/no-follow, cross-run CAS, and composite crash recovery.

### Body candidates: two-phase overlay plus lint

A body capability uses `StdinDelivery::AtomicOverlay` for one host-rebound frozen task file, but the trusted child sees a candidate in its run-owned staging root rather than committing live bytes. It emits a prepared body/envelope manifest. The parent commits the body only after the same clean-exit/output/reap checks. Non-zero, overflow, timeout, cancellation, teardown, or operational lint failure discards preparation and leaves/restores the prior body. Observe policy findings may still commit a structurally valid body.

### Durable receipt and R2a interruption semantics

R2a writes a final receipt only after parent-observed clean completion and final-byte verification. It uses run-owned receipts for ordinary resume skips.

If the process dies with only prepared/staged state or an incomplete live publication, R2a does not adopt or rerun it as accepted. Resume marks the phase operationally incomplete, restores a complete backup when deterministically available, or requires explicit operator cleanup/restart from the phase. No partial state is a pass.

R2b adds composite crash journals, exact committed-receipt adoption without rejudging, and automatic recovery at every boundary.

### Shadow and envelope idempotency

Gate invocations carry a stable invocation ID. Shadow records/envelopes include it, letting R2a detect duplicate evidence during one-run resume. R2b adds cross-process OS-locked idempotent append/replay.

### No duplicate validators

The existing commands remain the only validators:

- `freeze-acceptance` owns serde/schema validation, semantic validation, one batched judge call, disposition, and composite contract/lock/pin publication;
- `freeze-skeleton` owns serde/schema validation, set/graph validation, predecessor integrity, disposition, and composite skeleton/lock/pin publication;
- per-file lint owns task parsing, frozen-field comparison, body checks, and disposition;
- task-set lint and requirements trace own final set validation.

`GateEnvelopeV1` serializes their existing typed findings; it is not another validator:

```text
GateEnvelopeV1
  schema_version = 1
  report
  policy_findings[]
    text
    subject
    source_path
    remediation_scope
  operational_error?
    kind
    text
```

`remediation_scope` is a closed enum (`CandidateArtifact`, `Skeleton`, `PrdInput`, `Body`, `InheritedPredecessor`, `Operational`). Every existing finding constructor used by R2 has one explicit mapping in the gate module. Unknown/missing scope, unknown operational kind, or unclassified constructor is operational. Routing never infers ownership from prose. Exhaustive serialization tests cover every constructor and preserve exact text.

A task-file gate envelope is phase-local: it contains parser/body/frozen-tuple findings plus `InheritedPredecessor` shadows, and explicitly excludes task-set coverage/trace findings. Set-level coverage runs only after the complete body population in Phase D. This removes the current wrapper behavior where `LintSource::TaskFile` also evaluates parent-directory coverage while sibling bodies may not exist.

Gate commands write the envelope through a host-owned declared side channel. Ordinary stdout remains the human report and stderr retains exact `[shadow]` diagnostics. A missing or malformed required envelope is operational.

## Provider-neutral authoring

### Raw outcome mode

Trusted fixed scripts may request a raw provider outcome:

```javascript
const authored = await w.agent("acceptance-author-1", {
  task: prompt,
  tier: "planner",
  resultMode: "rawOutcome"
})

// { content, stopReason }
```

The fixed decomposition run injects a host-owned, non-empty read-only tool policy. The required production names are `Read`, `Grep`, `Glob`, and `CartographerScan`; `LeannSearch` is added only when the production registry actually exposes it. The launcher fails before the first author dispatch if a required name is absent after registry filtering. Every granted tool must be classified as read-only/approved external search. Bash, Write, Edit, and lock/provenance-writing tools are absent. The script cannot override the policy, and an empty allowlist is forbidden because current executor semantics treat empty as defaults.

The model receives repository/PRD read access sufficient to verify paths, commands, tests, and source. Publication remains host-owned.

The script checks typed `stopReason` before using `content`. Token-budget/max-token completion, malformed envelopes, missing content, timeout, cancellation, and provider failure are never parsed or repaired as partial JSON.

The host serializes real Rust values to construct prompt exemplars. Drift tests prove the real validators accept the exemplar and reject a discriminating invalid counterpart.

### Configured-only provider route

Fixed-decomposition author and judge calls use a provider client factory with `ProviderEndpointPolicy::ConfiguredOnly`. It does not consult ambient `ANTHROPIC_BASE_URL`. The run persists provider/profile identity and configuration source, never secret values. Hostile parent endpoint variables cannot redirect author prompts or repository excerpts.

### Author timeout and attempt accounting

Artifact authoring allows six logical attempts and body authoring ten, including initial attempts. Each call has a 1,500-second backstop. Typed transient transport retries remain within the logical attempt and never extend that backstop.

`AuthorBackstopExpired`, truncation, malformed outcome, and candidate rejection consume the attempt. `ControlPaused`/`ControlCancelled` record interruption and do not advance it; resume restarts the same logical attempt. R2a persists attempt number/state at phase boundaries and supports the proof's orderly pause/resume. R2b adds crash-active `AuthorDispatchLedger`, active-time heartbeats, and result-prepared recovery without duplicate provider work.

## Fixed decomposition meta-script

The script is embedded, hand-authored, immutable per binary version, provider-neutral, v3-only, and generic across PRDs, repositories, languages, and domains. Models may author artifacts, never script structure.

A decomposition launch requires `gate_mode=observe` or `enforce`. When mode is `off`, the launcher returns before run creation, path publication, provider construction, or author dispatch:

```text
decomposition requires workflow.gate_mode=observe or enforce; off evaluates and publishes nothing — set observe and retry
```

All R2 live proofs use observe.

### Phase 0 — PRD input integrity

Before the first author dispatch, the launcher invokes the same shared PRD identity/obligation kernel used by `freeze-acceptance`. This is one kernel, not a second validator. Malformed, duplicate, or zero obligations are `PrdInput` defects and stop with the exact manual remedy. Read-only artifact authors are never asked to repair the PRD.

### Phase A — acceptance

1. Classify any existing acceptance chain. A chain carrying a committed receipt for this run is resume-eligible. In R2a, a pre-existing portable chain without an R2 receipt cannot be adopted or silently skipped; the launcher requires a fresh destination or re-freezes through A. R2b owns explicit `AdoptedPredecessorReceipt` import.
2. If the run-owned receipt and integrity agree, skip authoring/freezing and preserve loud predecessor shadows.
3. Otherwise announce attempt/model-call-in-flight.
4. Author opaque contract candidate bytes, at most six logical attempts.
5. Invoke the declared acceptance-candidate/freeze capability with process stdin.
6. Route the typed envelope: `CandidateArtifact` may retry; `PrdInput` and `Operational` stop immediately.
7. Feed exact candidate findings and rejected content into the next attempt.
8. Persist `accepted` or run-metadata-only `accepted_with_shadow_findings`.

The existing freeze command checks stop reason before parsing the batched judge response. Provider failure, judge timeout, malformed output, missing/duplicate/unknown IDs, and token truncation publish nothing and are operational.

### Phase B — skeleton

1. Require a valid acceptance predecessor.
2. Validate and skip an existing full frozen skeleton only with a committed receipt for this run. R2a otherwise re-freezes; R2b owns explicit predecessor adoption.
3. Author the whole skeleton as one candidate, at most six logical attempts.
4. Invoke the declared skeleton-candidate/freeze capability.
5. Route the typed envelope: candidate-local `CandidateArtifact`/`Skeleton` may retry; `InheritedPredecessor` remains loud, linked, non-retrying, and nonblocking under observe; `PrdInput` and `Operational` stop immediately.
6. Feed exact retryable typed findings into the next attempt; never ask the skeleton author to repair predecessor findings.
7. Persist `accepted` or run-metadata-only `accepted_with_shadow_findings`.

### Phase C — task bodies

1. Host re-read and validate the frozen skeleton.
2. Fan out over frozen entries under the configured v3 concurrency cap.
3. Rebind every task ID/filename to the host-read frozen tuple.
4. Author one opaque body candidate, at most ten logical attempts.
5. Atomically overlay and run authoritative per-file lint.
6. A frozen-field mismatch in the staged candidate itself is `CandidateArtifact`/`Body`: discard preparation, feed the exact frozen value back, consume the attempt, and retry. It is not external mutation.
7. On other `Body` policy findings, feed exact text back and retry.
8. `InheritedPredecessor` findings from valid observe-stamped acceptance/skeleton freezes remain loud, are linked into the body/run shadow metadata, and never requeue or block the body in observe.
9. Mutation of an already accepted body/current frozen chain, or any new `Skeleton`/`PrdInput` finding not caused by the candidate, stops as operational/external mutation. Set-level coverage cannot appear in a Phase C envelope.
10. On exhausted policy-only body retries in observe, retain the last structurally valid committed body and mark the body record `accepted_with_shadow_findings`.
11. On operational/integrity failure, stop that body and the phase.

### Phase D — typed set gates

1. Compute and persist one canonical set-gate input manifest: PRD digest, acceptance/skeleton/lock/pin digests, every sorted task path+digest, and every evidence/index input declared by the gate.
2. Run task-set lint bound to that manifest.
3. Run requirements trace bound to that manifest.
4. Consume their typed `GateEnvelopeV1` findings.
5. Route by remediation scope; never infer ownership from report prose. Read-only gate call identity and reuse include the complete manifest digest, and postcondition rechecks every input before reuse.

Current finding taxonomy:

| Scope | Finding families | R2 action |
|---|---|---|
| Skeleton | unclaimed obligations, phantom citations, zero task citations, dependency/edge defects, frozen deliverable-contract defects | shadow-mark run and continue |
| Inherited predecessor | acceptance/skeleton freeze already carries observed findings | preserve loud linked shadow; no retry and no block under observe |
| PRD input | malformed, duplicate, or zero PRD obligations | stop in every mode with manual PRD remedy |
| Operational | unreadable/malformed task population, corrupt freeze, graph-lowering failure, shadow-log/write failure | stop in every mode |
| Body | none after successful C | first appearance means external mutation/invariant failure; stop operationally |

D has no body-remediation loop.

In observe, set-level policy findings produce a run-level `accepted_with_shadow_findings` and Phase E proceeds.

Future enforce semantics are designed but not enabled in R2:

- skeleton findings may route D → B once;
- B publishes a fresh skeleton freeze with anti-laundering provenance;
- all bodies are revalidated, and only mismatches re-enter C;
- set gates rerun once;
- a second skeleton failure aborts with exact findings;
- PRD input and operational findings never enter D → B.

### Phase E — reconciliation and summary

1. Re-read the full frozen chain.
2. Reconcile every frozen ID/filename with exactly one landed body.
3. Verify no body changed a frozen tuple.
4. Verify every accepted call identity, publication receipt, exact output digest, subject disposition, and postcondition still holds.
5. Require every skipped phase/body to have terminal subject metadata `accepted` or `accepted_with_shadow_findings`; a receipt published for an intermediate rejected/retry candidate cannot complete a subject.
6. Persist the final decomposition summary and terminal status.

`accepted_with_shadow_findings` exists only in decomposition run/phase/body metadata with finding count and durable event references. It is not added to artifact schemas, locks, pins, or runtime task statuses. Exact findings remain canonical in durable events and the R1 shadow JSONL log.

## Resume model

R2a extends ordinary v3 persisted call records/checkpoints with fixed decomposition phase/body outcomes. Resume never trusts file existence alone; it validates run binary/script/catalog identity, ordinary call state, existing host freeze/lint kernels, run-owned receipt exact bytes, terminal subject disposition, and current postcondition.

| State | R2a resume behavior |
|---|---|
| Valid run-owned acceptance receipt + terminal outcome + chain | skip A |
| Partial/corrupt/incomplete acceptance publication | stop operationally; never adopt |
| Valid run-owned skeleton receipt + terminal outcome + full chain | skip A and B |
| Draft/incomplete skeleton | resume or restart B after explicit cleanup |
| Body receipt/postcondition + terminal body outcome + lint accepted | skip body |
| Missing/mutated body | requeue body |
| Incomplete/stale set gates | resume D |
| Orderly paused/cancelled author dispatch | retry same logical attempt |
| Binary/script/catalog mismatch | refuse with named remedy |

R2a permits one active executor per run and launcher-refuses another active run targeting the same task root. Existing store locking plus single-owner execution is sufficient for the controlled proof path. A process crash releases ownership; resume normalizes stale `Running` records and continues from the last durably completed phase, but incomplete publication may require explicit cleanup.

R2b adds canonical task-root writer leases across run IDs, OS-locked generation-CAS state/checkpoint transactions, crash-active ledgers, composite publication adoption, and full automatic crash recovery.

## Progress, TUI, CLI, and `.decompose.log`

### Command surface

```text
/workflow decompose --prd <PATH> --tasks <DIR>
archon workflow decompose --prd <PATH> --tasks <DIR> --yes
```

The launcher persists the run and returns its run ID before background execution starts. Existing lifecycle commands remain canonical:

```text
workflow status <run-id>
workflow pause <run-id>
workflow resume <run-id>
workflow cancel <run-id>
```

No `continue` alias is added.

A TUI-launched run retains its executor join handle and cancellation token. Orderly TUI closure signals cancellation, waits for active trusted child cleanup/reaping, and does not accept the run. Abrupt process death releases OS ownership; next resume normalizes stale `Running` records and may require cleanup of an incomplete publication. The `--yes` CLI form is the long-running alternative.

### R2a durable event writer

The one active R2a executor owns ordered decomposition event emission. For each transition it appends the normal durable workflow event, appends/flushes `.decompose.log`, then enqueues transient TUI/CLI delivery. Stable event IDs are persisted so orderly resume does not repeat completed transitions.

Pause/cancel continue through existing lifecycle state and events; R2a proof commands are serialized by the coordinator and do not run concurrent state writers. R2b adds one cross-process OS-locked sequence+append transaction, generation-CAS state updates, and a durable progress outbox that recovers crashes between event/log/transient delivery.

### Log contract

The log path is:

```text
tasks/<PRD>/.decompose.log
```

It is append-only across resume and begins with run ID plus binary/script/catalog identities. Each resume appends a resume marker. It records phase banners, author attempts, model-call-in-flight announcements, host stages, body verdicts, retries, exact findings, and final summary.

It excludes prompts, candidate artifact bodies, environment values, and secrets. One progress event over 64 KiB is operational rather than silently truncated. Stable event IDs make duplicate replay lines identifiable.

R2a launcher policy permits one active decomposition for a task root, so one executor owns the log.

### UI backpressure

Durable event/log persistence never awaits a congested UI indefinitely. Transient delivery uses a separate bounded drain. If its queue fills, progress coalesces and later emits a visible marker directing the operator to durable logs. A closed UI cannot stall or accept a run.

CLI decomposition drains and prints events instead of discarding them.

### Status detail

`workflow status` adds:

- run kind/template version;
- starting binary revision/hash plus script/catalog digests;
- current phase and subject;
- logical attempt, remaining active timeout, and budget;
- active provider/model and elapsed time;
- active host capability ID;
- accepted/interrupted/failed call counts;
- body totals: pending, accepted, accepted-with-shadows;
- unresolved shadow count;
- last operational error;
- `.decompose.log` path;
- phases/bodies eligible for resume skip;
- finalization/observer state when applicable.

## Observe-only run-end acceptance observer

### Recoverable terminal finalization seam

All workflow terminal paths are centralized under a durable finalizer. Script-internal terminal event emission is removed from the authoritative path. The finalizer owns this order:

```text
compute terminal summary
→ under run lock persist terminal state + FinalizationRecord
→ append the stable terminal event through the locked event transaction
→ persist terminal_event_committed
→ run optional observer
→ persist observer completed/failed
```

At implementation-run launch, the host performs a non-blocking opt-in snapshot over the canonical task root. It does not validate or require a freeze and cannot reject admission:

- whole chain absent: omit the backward-compatible optional eligibility field; absence decodes as `LegacyAbsent`;
- any freeze-chain artifact present: persist `observer_eligibility = Expected`, canonical task-root identity, expected artifact-path set, and the observed portable acceptance identity/digests when readable.

This snapshot is authority for end-of-run observer eligibility. A run that launched `Expected` cannot become legacy-silent by deleting or renaming every artifact; later absence is observer-operational. An omitted eligibility field decodes as `LegacyAbsent` and preserves byte-identical legacy serialization even if unrelated artifacts appear later.

`FinalizationRecord` records terminal-state commit, terminal-event commit, and observer intent/state derived from the persisted launch snapshot. `Expected` includes durable `observer_pending` at terminal-state persistence; an omitted/`LegacyAbsent` state adds no observer field.

R2a persists terminal state before event and runs the observer in the same single-owner finalizer. An orderly retry can complete a pending observer without rerunning implementation. R2b adds startup-wide crash reconciliation, exclusive `observer_claimed` leases, and finalization-only recovery after arbitrary process death.

Observer eligibility is a closed table:

- normal implementation run with terminal `Completed`/accepted/noop/needs-review outcome: eligible according to launch snapshot;
- `FixedDecompositionV1`: never eligible, even though it creates a freeze chain;
- paused, cancelled, failed, blocked, or still-running implementation runs: not evaluated until/if they later reach an eligible terminal completion;
- analysis/planning/saved-script kinds: ineligible unless a future explicit versioned opt-in is added.

Tests prove state persistence precedes terminal event and observer events for every terminal path, and prove the closed eligibility table. The trading decomposition terminal state never creates observer intent.

### Opt-in and legacy silence

The observer derives the canonical task root from persisted run/task-universe metadata and consults the launch-time eligibility snapshot.

- `LegacyAbsent`: return without filesystem probing, output, event, shadow line, or observer field. Legacy serialized behavior is unchanged.
- `Expected`: validate the complete required chain and compare it with the launch-time task-root/artifact identity after terminal persistence.
- Missing-all, partial, corrupt, mismatched, replaced, or stale chain for an `Expected` run: record observer operational failure; terminal status remains unchanged.

### Authority pin

R2 hard-pins observer authority to `ObserveOnly`, independent of `workflow.gate_mode`. It may append run-end shadow findings and observer operational records but cannot alter terminal status, exit disposition, or acceptance outcome even when global gate mode is `enforce`.

A test fixes `gate_mode=enforce`, supplies a failing frozen acceptance check, and proves terminal outcome unchanged, terminal persistence before observer events, run-end shadow JSONL written, and no blocking authority available. R4's promotion is a deliberate visible policy diff after evidence review.

### Acceptance check execution boundary and R2a deferral

R2a evaluates only `AcceptanceCheck::Floor` contracts whose `typed_verifier_command` is absent. The existing deliverable-verifier generator is refactored to extract one shared pure `evaluate_declarative_floor` kernel over artifact/record fields. The existing verifier path and run-end observer both call that kernel; only the existing verifier path then renders shell for command-bearing work. This is kernel extraction, not a parallel validator.

Model-authored `AcceptanceCheck::Command`, a floor containing `typed_verifier_command`, and residual-gap `fail_closed_check` text never reach host `hostCommand` argv, a host shell, or a permissive Bash registry path. In R2a they are not executed. The observer records the explicit post-terminal operational deferral and leaves terminal status unchanged.

R2b implements the preserved hardening design: a mandatory ephemeral isolated acceptance world with read-only project/repository/task mounts, scratch-only writable tmpfs, network disabled, empty environment, no provider credentials/host sockets/devices/daemon endpoints, bounded resources/output, stdin-delivered command text, and whole-world teardown. If containment cannot be proved, there remains no host fallback.

### Residual-gap semantics

R2a validates `acceptance-residual-gaps.json` structurally whenever it exists, including all-pass plus stale/extra-gap rejection, using the existing `validate_residual_gaps` kernel. Because every residual record includes command-bearing `fail_closed_check`, R2a records observer-operational deferral rather than executing or accepting gap coverage. Uncovered failed criteria still produce run-end policy shadows; terminal status remains unchanged.

R2b adds isolated execution and then requires exactly one authorized valid record plus a passing isolated fail-closed check for each covered failed permitted criterion; supplementary, passing, unknown, duplicate, or non-permitted coverage remains invalid.

## Typed event vocabulary

Meaningful transitions produce both durable and transient representations:

```text
DecompositionPhaseStarted
AuthorAttemptStarted
ModelCallInFlight
AuthorAttemptInterrupted
AuthorAttemptRejected
HostCommandStarted
HostCommandCompleted
ShadowFindingsObserved
SubjectAccepted
SubjectAcceptedWithShadowFindings
DecompositionPhaseCompleted
DecompositionCompleted
RunEndAcceptanceObserverStarted
RunEndAcceptanceShadowObserved
RunEndAcceptanceObserverFailed
```

Provider/model identity is populated by runtime resolution after dispatch, never hardcoded in the script.

## Error taxonomy

Operational/integrity failures stop in every mode:

- invalid CLI/config/run arguments;
- protected-path or canonical-boundary violation;
- undeclared command ID or invalid host token;
- input/output/progress-event limit overflow;
- spawn, timeout-enforcement, cancellation, or reap failure;
- candidate transaction or rollback failure;
- malformed/truncated author or judge outcome;
- malformed/duplicate/zero PRD obligations;
- unreadable/malformed task population;
- corrupt/mismatched/stale lock, pin, digest, provenance, or postcondition;
- missing/malformed typed gate envelope;
- durable event, shadow JSONL, state, or `.decompose.log` write failure.

Policy findings follow decomposition gate mode and never acquire run-end authority in R2:

- acceptance/skeleton policy defects;
- per-file lint findings;
- set coverage/edge/contract findings;
- judge-refuted checks;
- predecessor freezes carrying observed findings;
- run-end unmet acceptance criteria, always observe-only in R2.

Policy-only retry exhaustion in observe records `accepted_with_shadow_findings` and continues. Stopping on those findings would smuggle enforcement through the meta-script. Operational failures never continue under that marker.

## Dry-run semantics

Dry-run parses and validates every host command request, records `HostCommand` calls in the plan, and never spawns a process or lands candidate bytes. The method-specific stub contains every result field read by the fixed script and marks `dryRun: true`.

Because a zero-exit stub explores only the success branch, dry-run reports that command-dependent remediation topology is incomplete. It is not evidence that non-zero/remediation branches are valid.

## Testing strategy

### `hostCommand` primitive

- Raw `w.hostCommand` wiring test; deleting the `w` call site must fail it.
- Method parse/as-string and exhaustive classification tests.
- Undeclared command ID rejected before spawn.
- Argument-boundary and shell-metacharacter tests proving no shell interpretation.
- Host token grammar, canonical-root, traversal, symlink, and frozen-tuple rebinding pairs.
- Host-computed identity and postcondition-gated reuse pairs.
- Non-zero exit returned as process data and selected remediation branch.
- Separate stdout/stderr preservation.
- Concurrent large-output drains without pipe deadlock.
- Stdin/stdout/stderr overflow as operational with no publication.
- Timeout, pause, cancel, and parent death terminate/reap every child shape reachable by the declared trusted commands; detached-command eligibility is refused.
- Pause/cancel preserve logical attempt number; timeout advances it.
- Dry-run records without spawn or write.
- Permission/admission/sandbox refusal before spawn.
- Hostile ambient/repository endpoint tests proving `env_clear`, exact named child profiles, trusted CLI/user author/judge routing, and rejection of unapproved project routes before content leaves the process.
- R2a starting binary revision plus script/catalog resume refusal and no-deploy-while-active protocol; R2b executable snapshot/handshake/live-replacement tests.
- Binary/script/catalog mismatch refusal.
- No agent or Bash-tool invocation in any decomposition host stage.

### Publication and meta-script

- Fake-client complete phase-order test.
- Empty destination reaches first acceptance author call rather than `NoTaskFiles`.
- Artifact exemplars serialized from real structs and accepted by real validators.
- Invalid counterparts rejected by the same commands.
- Required author tools resolve to exact production registry names; missing/filtered tools fail before dispatch and no empty/default fallback exists.
- `gate_mode=off` refuses decomposition before run creation, path reads, or provider construction.
- Phase 0 and every envelope route malformed/duplicate/zero PRD obligations to immediate `PrdInput` failure, never author retry.
- Typed author backstop, transport, pause, cancel, truncation, and malformed-outcome accounting respects one persisted active-time budget.
- R2a two-phase prepare/parent-commit freeze publication: overflow/nonzero/timeout after child preparation publishes nothing; deterministic rollback/refusal covers controlled interruption boundaries.
- R2a two-phase body preparation/parent commit and deterministic rollback/refusal.
- R2b composite crash recovery and committed receipt adoption reconstruct the call record without a second judge call.
- Accepted reuse requires exact receipt-produced contract/body, lock, pin, envelope, and stable shadow-record membership digests; later JSONL appends do not invalidate prior receipts and a different valid freeze is not reusable.
- R2a requires fresh/re-frozen run-owned receipts and refuses silent adoption of pre-existing chains; R2b tests explicit adopted-predecessor import.
- Read-only set-gate identity/postcondition binds the complete PRD/freeze/task/evidence input manifest.
- R2a launcher refuses a second active task-root run; R2b writer-lease/CAS races.
- R2a trusted-child staging enumeration, exact write-set match, receipt, and pre/post mutation sentinel tests; R2b dirfd/no-follow/OS-confinement tests.
- Batched acceptance judge and stop-reason-before-parse tests.
- Bodies cannot begin before both freezes; call-site sabotage proves sensitivity.
- Frozen token mutation and symlink/path escape rejected before spawn.
- Exact finding returned to the responsible author.
- Observe policy exhaustion continues with run-metadata marker.
- Operational exhaustion stops and cannot create that marker.
- Phase B/C/D envelope tests preserve `InheritedPredecessor` as loud, linked, non-retrying, and nonblocking under observe; B never asks the skeleton author to repair its predecessor.
- Phase C tests exclude set-level coverage during partial population, distinguish staged candidate frozen-field mismatch from external accepted-artifact mutation, and sabotage the call site to prove wiring.
- Phase D typed-router tests: skeleton, inherited predecessor, PRD input, operational, and external body mutation.
- No dead body-remediation path.
- Future D → B route limited to one cycle, tested but not enabled.

### Persistence, UI, and resume

- Run persisted and run ID returned before executor spawn.
- R2a one-executor run ownership and launcher refusal of a second active task-root run; R2b canonical task-root writer lease and stale-owner recovery.
- R2a TUI shutdown retains the executor handle, signals orderly cancellation, and reaps trusted host work; R2b abrupt-parent-death supervisor recovery.
- Happy-path and refusal/requeue durable event sequences.
- R2a single-owner durable event→log→transient ordering and orderly resume deduplication; R2b cross-process locked sequence+append.
- R2a single-owner phase/call checkpoint persistence; R2b cross-process generation-CAS state/checkpoint races.
- R2a single-owner finalizer ordering; R2b observer claim/stale recovery.
- R2a bounded logical-attempt and pause/no-advance proof; R2b crash-active dispatch-ledger boundaries.
- R2b progress-outbox crash replay.
- Model-call-in-flight announcement.
- UI queue saturation/coalescing without run stall.
- TUI closure terminates executor, releases lease, and resumes cheaply.
- CLI prints rather than discards progress.
- `.decompose.log` append/resume markers, exact findings, sanitization, and flush failure.
- Status of incomplete, interrupted, shadowed, and terminal decomposition runs.
- Resume skips only with four-way agreement; mutation invalidates skip.

### Run-end observer

- Launch-time `LegacyAbsent`: no evaluation, output, event, observer field, or shadow record.
- Launch-time `Expected`: deleting every artifact, partial chain, or identity replacement records observer-operational after terminal persistence and cannot become legacy-silent.
- Central finalizer proves state commit → terminal event commit → observer intent/completion for every terminal path.
- R2a orderly terminal state-before-event-before-observer proof; R2b arbitrary-crash finalization-only recovery.
- Failing criterion under global enforce: shadow written, terminal unchanged.
- R2a pure-floor evaluation produces shadows normally; command checks, nested floor verifiers, and residual fail-closed checks record the exact post-terminal operational deferral with terminal status unchanged.
- R2a validates residual-gap structure whenever the file exists, including all-pass+stale-gap, but never accepts command-bearing coverage.
- Frozen command text never reaches `hostCommand` argv or a host shell.
- R2b hostile-world tests prove command/nested-verifier/fail-closed text cannot mutate a host sentinel, read parent secrets, reach network/daemons, or leave descendants after teardown.

### Legacy tripwires

- Existing observe/enforce mortal admission test remains green.
- Temporary sabotage at `execute_generated_v2_run` remains sensitive and uncommitted.
- Existing lifecycle E2E fixtures remain freeze-unaware.
- No runtime admission prerequisite is added for legacy task sets.

## Proof package 1 — synthetic full lifecycle

Create a clean scratch Git project outside protected paths containing:

- one generic PRD;
- two or three trivial canonical tasks;
- scratch-only target files;
- a fresh task root whose acceptance chain is produced by the R2a decomposition;
- one host-serialized commandless floor exemplar whose unmet artifact lies outside task write ownership.

Proof sequence:

1. Decompose through the first-class R2 surface under observe.
2. Freeze acceptance and skeleton.
3. Pause during an author call and prove the logical attempt number does not advance.
4. Inspect durable status and `.decompose.log`.
5. Resume and prove accepted phases/bodies skip while interrupted work retries.
6. Complete every body and set gate.
7. Launch the generated scratch tasks through a normal v3 implementation run.
8. Reach and persist terminal status.
9. Prove terminal persistence/event precedes observer events.
10. Before launch, host-assert the frozen synthetic criterion exactly matches the commandless floor exemplar; refuse the proof if the model selected `Command` or a nested verifier. Prove the unmet floor creates run-end shadow evidence.
11. After decomposition is complete, run a focused synthetic observer authority probe with global `gate_mode=enforce`; no decomposition launch uses enforce, and terminal outcome remains unchanged.
12. Prove no undeclared host capability or decomposition agent Bash invocation occurred.

A false-green implementation outcome is evidence in R2, not permission to weaken findings or promote enforcement.

## Proof package 2 — trading PRD decomposition only

Preconditions:

- R2 implementation and independent review complete.
- One local R2 commit exists before release build.
- No active Archon workflow or Cargo/rustc/test process before Cargo.
- Release is built from a clean worktree/snapshot of that exact commit, excluding protected dirty WIP and any post-commit non-protected edits; alternatively a complete content-addressed source manifest proves every compiled input.
- Release built after commit and atomically deployed to both executable paths.
- Both binaries report HEAD and equal hashes.
- Provider route is probed live; port claims are not assumed.
- Protected trading WIP and target directory are snapshotted.

Run through the first-class TUI surface with all decomposition gates observing. The reviewing operator—not Steven—launches and monitors the proof after synthetic clearance.

Evidence package contains:

1. run ID, run kind, template version, and script/catalog digests;
2. acceptance freeze and provenance;
3. skeleton freeze and provenance;
4. deliberate pause/resume with valid predecessor skip;
5. every body attempt and authoritative per-file lint outcome;
6. task-set lint and requirements trace envelopes;
7. exact shadow findings and bounded remediation history;
8. final frozen/body reconciliation and task population;
9. durable terminal decomposition status;
10. `.decompose.log`;
11. R1 shadow JSONL records;
12. false-positive classification and unresolved findings;
13. unchanged legacy mortal-admission evidence;
14. unchanged protected trading tree.

No trading implementation workflow is launched. After delivering both packages, R2 stops for independent evidence review. No promotion, R3, or implementation launch follows automatically.

## Cargo and process discipline during implementation

Subagents are used for isolated implementation components and independent reviews. Subagents do not run Cargo.

One coordinator serializes every Cargo invocation in the repository:

1. require the exact Archon workflow-process guard to be zero;
2. inspect existing Cargo, rustc, linker, and Rust-test processes;
3. if one exists, identify its owner, CPU, descendants, and log before waiting;
4. never launch a second Cargo process in the same target directory;
5. separate compile from test execution (`--no-run` then direct harness where practical) so test watchdogs do not consume compile time;
6. give every long command a watchdog covering all terminal states;
7. on silence, inspect CPU, descendants, open files, and logs rather than waiting indefinitely;
8. terminate only process trees launched by the coordinator;
9. never kill protected services or unrelated scanners/daemons.

## Implementation ownership map

Implementation planning may refine exact filenames, but responsibility remains separated:

- `archon-workflow`: host method/request/result contracts, raw `w.hostCommand`, dry-run semantics, fixed script source, typed phase metadata.
- binary workflow runtime: catalog construction, token rebinding, process adapter, environment profiles, transaction journal, live dispatch, lease, progress reporter.
- workflow command surfaces: decompose action, fixed-run launcher, CLI/TUI streaming, status rendering.
- existing gate commands: candidate consumption, typed envelope serialization, authoritative validation/publication.
- run finalization: post-terminal optional acceptance observer with immutable R2 observe-only authority.
- tests: focused modules by primitive, transaction, meta-script, lifecycle, observer, and live proof harness.

No file should exceed the repository's 500-line growth guard; split by responsibility before crossing it.

## Stop conditions

Stop R2 implementation or proof immediately if:

- `hostCommand` requires model-authored executable/argv/cwd/environment values;
- token authority cannot be rebound to host-read frozen artifacts;
- a command bypasses normal persisted call records/checkpoints;
- a trusted built-in cannot stage a complete declared output set for parent verification, or mutation sentinels detect an undeclared host effect;
- R2a cannot enforce no-deploy-while-active and binary/script/catalog resume identity;
- R2a launcher permits a second active decomposition for the same task root;
- a declared direct command can detach or cannot be bounded/reaped for every reachable child shape on the target platform;
- raw acceptance/fail-closed/nested-verifier command text could execute on the host, or R2a does anything other than record the explicit operational deferral;
- gate commands would be duplicated by a parallel validator;
- an artifact must be published from truncated/malformed output;
- a legacy run consults a freeze prerequisite;
- R2a terminal finalizer cannot persist state before terminal event and observer;
- run-end observer logic executes before terminal persistence or can alter R2 terminal status;
- a launch-time `LegacyAbsent` run emits any observer output, or a launch-time `Expected` run can erase/mutate its chain and return silently;
- protected trading files would be edited, staged, committed, or implemented;
- a second Cargo process would contend for the repository target directory;
- the synthetic proof does not complete before the trading proof;
- provider reachability or deployed revision/hash cannot be proven;
- bounded retries exhaust on an operational failure.

## R2a completion and R2b handoff

R2a is complete only when:

- the full implementation satisfies this spec and receives independent code review;
- primitive, phase, persistence, UI, resume, observer, and legacy tripwire suites pass;
- the synthetic full-lifecycle proof package is complete;
- the trading decomposition-only proof package is complete;
- all gates remain observing by default and run-end authority remains `ObserveOnly`;
- shadow evidence is reviewed and apparent false positives are classified;
- protected trading WIP is unchanged;
- one local R2 commit is built after commit and atomically deployed to matching binaries;
- no push or CI trigger occurred;
- work stops for R2a evidence review before R2b, R3, R4 promotion, or any trading implementation run.

After R2a evidence review passes, R2b implements the deferred isolated acceptance world and explicit predecessor adoption without redesigning or re-opening the proven spine. R2b receives its own focused implementation plan and verification, then stops again before any promotion.
