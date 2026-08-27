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

A fixed decomposition run persists:

- run kind and template version;
- exact embedded script source and BLAKE3 digest;
- canonical serialized script arguments and digest;
- canonical command catalog and digest;
- starting binary revision, SHA-256, and BLAKE3 identity;
- a run-owned executable snapshot path plus its inode/identity metadata;
- canonical project, PRD, and task-root paths;
- the complete capability write-set policy and protected-path snapshot;
- `.decompose.log` path;
- decomposition phase/body records;
- progress event sequence/outbox state.

`WorkflowRun` remains engine-neutral. Generated-run metadata owns fixed-template identity.

### Binary pin and live replacement safety

Before the run is persisted, the launcher copies `current_exe()` to an atomic, fsynced, non-symlink executable snapshot inside the run directory. It records the embedded revision plus SHA-256/BLAKE3 hashes. Every built-in R2 capability uses this snapshot rather than the mutable installed path. Before every spawn, the executor reopens the snapshot without following symlinks and verifies regular-file identity, revision, and hashes. A mismatch is operational before spawn.

A host-configured external program is ineligible unless its absolute path and content hash were fixed in the catalog at launch and match immediately before each spawn. R2's built-in catalog uses only the run executable snapshot. PATH lookup and bare `archon` are forbidden.

This prevents an atomic deployment from mixing old parent runtime code with new child command behavior while a run is live.

Resume requires the invoking binary's revision/hash, embedded script digest, and embedded catalog digest to match the persisted starting values before executing anything. A mismatch refuses resume:

```text
this run was launched by binary revision W with script digest X and catalog digest Y; relaunch under the current binary, or finish it with the binary that started it
```

Resume never silently substitutes a persisted or current version. The run-owned child snapshot does not authorize a new parent runtime to resume an old run.

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

Every `declared_write_set` entry identifies an exact path or tightly bounded append target derived from host-owned tokens. Built-in capabilities declare all effects, including contract/skeleton files, lock, host pin, body file, gate-envelope file, transaction/receipt files, and R1 shadow JSONL. `.decompose.log`, run events, and run state remain parent-owned writes outside the child capability.

The capability executor constructs an execution world that permits only the declared write set. A command attempting another write is operational. The catalog's serialized write set is part of its digest and status review surface.

Only trusted host launchers/templates may supply a catalog. Model-authored and ordinary authored scripts cannot declare capabilities. Calling an undeclared `commandId` fails before spawn.

### Token authority and validation

Every dynamic argv token declares both its host source and validator. Script-provided token values have no authority.

- `ProjectRoot`, `PrdPath`, and `TaskRoot` equal launcher-owned canonical paths.
- Paths remain within the declared root, reject traversal, and reject symlinks/symlink escape.
- Existing path components are opened relative to retained canonical directory handles with no-follow semantics.
- An absent leaf is resolved through a retained canonical parent handle.
- `FrozenTaskId` matches canonical `TASK-<AREA>-<NNN>` grammar.
- `FrozenTaskFile` is a direct `TASK-*.md` child of the canonical task root.
- Task ID and filename equal the tuple obtained by the host re-reading the serde-validated frozen skeleton on disk.
- Immediately before publication/spawn, parent directory identity and expected old-content digest are rechecked under the task-root writer lease.
- Any mismatch is operational before candidate publication or spawn.

The script may iterate model-returned skeleton data for scheduling, but host rebinding prevents those bytes from becoming argv authority.

### Host-computed call identity and produced-output binding

The executor computes:

```text
BLAKE3(
  "host-command-v1"
  || command_id
  || catalog_digest
  || run_executable_digest
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

Only after the parent supervisor observes zero exit, bounded and fully closed stdout/stderr, no timeout/interruption, successful child and supervisor reaping, a valid gate envelope, and the complete provisional manifest does the parent commit. Under the task-root lease, it performs CAS rechecks, atomically renames the entire output set, fsyncs files/directories, then mints/fsyncs the final `PublicationReceipt` and accepted call record.

The acceptance transaction covers final judged contract bytes, acceptance lock, host pin, gate envelope, and stable shadow records. The skeleton transaction covers skeleton, skeleton lock, updated host pin, gate envelope, and stable shadow records. Every target, prior digest, temporary path, backup path, and final digest is registered before staging.

Publication uses retained canonical directory handles, no-follow opens, `renameat`-style operations, parent identity checks, file fsync, directory fsync, and compare-and-swap. Path-based validate-then-rename is forbidden.

A task-root-wide OS writer lease remains held for prepare/commit/rollback. Another run targeting the same canonical task root cannot begin or resume as a writer while it is held.

### Body candidates: two-phase overlay plus lint

A body capability uses `StdinDelivery::AtomicOverlay` for one host-rebound frozen task file, but the child sees a staged candidate in its confined world rather than committing live bytes. It emits a prepared body/envelope manifest. The parent commits the body only after the same clean-exit/output/reap checks. Non-zero, overflow, timeout, cancellation, teardown, or operational lint failure discards preparation and leaves/restores the prior body. Observe policy findings may still commit a structurally valid body.

### Durable receipt, crash rollback, and adoption

Crash recovery never re-executes a nondeterministic committed freeze merely because the call record is missing:

- prepared but not parent-committed: roll back/discard before phase selection;
- parent-committed final receipt with every exact output/postcondition matching: reconstruct/adopt the accepted call record without rerunning the judge;
- committed receipt with output mismatch: operational integrity failure;
- incomplete/unresolvable journal: operational failure naming every target, temporary, and backup path.

A crash before parent commit cannot turn prepared bytes into accepted state. A crash after parent commit preserves the exact judge result, provenance timestamp, and final bytes. Recovery never mints a different judgment under the same identity.

### Shadow and envelope idempotency

Gate invocations carry a host-generated stable invocation ID. Shadow JSONL and typed gate-envelope writes include that ID and use an OS-locked idempotent append/write path. A crash after shadow evidence but before publication may retry without duplicating evidence under a new identity.

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

Artifact authoring allows six logical attempts, including the initial attempt. Body authoring allows ten per body, including the initial attempt.

Each attempt owns a durable `AuthorDispatchLedger`: logical attempt, dispatch ID, active-time consumed, remaining deadline, transport retry, state (`Prepared`, `InFlight`, `ResultPrepared`, `Committed`, `Interrupted`, `Expired`), and result digest. `Prepared` is fsynced before provider dispatch; active time is charged against a persisted monotonic deadline/heartbeat; result bytes/digest become `ResultPrepared` before phase state changes. Crash reconciliation charges elapsed active time through the last trusted heartbeat/deadline, never grants a fresh budget, and never re-dispatches a `ResultPrepared` response. Unknown stale in-flight work becomes interrupted at the same attempt with the remaining persisted budget.

Each logical attempt has 1,500 seconds of active wall-clock budget. Pause time is excluded and remaining active budget is persisted. Up to six typed transient transport retries may occur within that budget; they do not reset or extend it.

The workflow port uses typed errors rather than matching timeout strings:

- `AuthorBackstopExpired` consumes and advances the logical attempt;
- `TransportTransient` retries within that attempt and remaining time;
- `ControlPaused`/`ControlCancelled` records interruption and does not advance;
- token-budget truncation and malformed outcome consume the attempt without parse/repair.

A logical attempt also advances after a syntactically/semantically rejected candidate, policy rejection requiring revision, or frozen-identity violation. Resume restarts an interrupted dispatch at the same logical attempt and remaining active budget.

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

1. Classify any existing acceptance chain. A chain carrying a committed receipt for this run is resume-eligible. A valid pre-existing portable chain without an R2 receipt is never silently treated as a completed call: either launch explicitly imports it into a host-signed `AdoptedPredecessorReceipt` after full integrity/provenance/current-binary validation, or A re-freezes it. Import is non-mutating, records exact input digests and predecessor shadows, and is available only by explicit launcher policy.
2. If receipt/adoption and integrity agree, skip authoring/freezing and preserve loud predecessor shadows.
3. Otherwise announce attempt/model-call-in-flight.
4. Author opaque contract candidate bytes, at most six logical attempts.
5. Invoke the declared acceptance-candidate/freeze capability with process stdin.
6. Route the typed envelope: `CandidateArtifact` may retry; `PrdInput` and `Operational` stop immediately.
7. Feed exact candidate findings and rejected content into the next attempt.
8. Persist `accepted` or run-metadata-only `accepted_with_shadow_findings`.

The existing freeze command checks stop reason before parsing the batched judge response. Provider failure, judge timeout, malformed output, missing/duplicate/unknown IDs, and token truncation publish nothing and are operational.

### Phase B — skeleton

1. Require a valid acceptance predecessor.
2. Validate and skip an existing full frozen skeleton only with a committed run receipt or explicit `AdoptedPredecessorReceipt`; otherwise re-freeze/import under the same policy.
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

Resume never trusts file existence alone. For each phase it requires agreement among:

1. persisted/current binary, script, and catalog identities;
2. normalized durable call state;
3. existing host integrity/validation kernels;
4. accepted call identity, committed publication/adoption receipt, exact produced-output/input-manifest digests, terminal subject disposition, and current filesystem postcondition.

| State | Resume behavior |
|---|---|
| Valid acceptance receipt/adoption + terminal subject outcome + contract/lock/pin | skip A |
| Partial/corrupt acceptance chain | stop operationally |
| Valid full skeleton receipt/adoption + terminal subject outcome + chain | skip A and B |
| Unlocked draft skeleton | resume B; never treat as frozen |
| Body receipt/identity/postcondition + terminal body outcome and authoritative lint accepted | skip body |
| Body with recorded shadows | re-evaluate lint; skip only if receipt/output/postcondition still agree, preserving loud shadows |
| Missing/mutated body | requeue that body |
| Incomplete/stale set gates | resume D |
| Stale `Running` call | persist interrupted/non-reusable, then rerun |
| Open publication journal | restore or adopt from committed receipt before phase selection |
| Binary/script/catalog mismatch | refuse with named binary remedy |

### Writer exclusion

A decomposition executor holds two OS-backed leases:

- a run execution lease keyed by run ID;
- a writer lease keyed by the canonical project/task-root digest.

The task-root lease excludes every other decomposition run targeting that root, including different run IDs and resume attempts. Publication additionally uses compare-and-swap against expected prior digests, so a stale writer cannot commit after losing/reacquiring a lease.

Process death releases leases. Resume reacquires both and normalizes stale running records. Status is read-only and takes neither writer lease.

### Locked state and checkpoint transaction

Every run-state, decomposition metadata, finalization metadata, call-record frontier, subject outcome, author ledger, and checkpoint mutation uses one OS-locked read-modify-write transaction with expected generation/CAS. The transaction writes/fsyncs temporary state, renames, fsyncs the run directory, and advances generation exactly once. Equal-generation concurrent writers are rejected/retried under the lock; direct unlocked load-modify-save is forbidden. Pause/cancel, fanout completion, reporter checkpoints, finalizer, and resume all use this primitive.

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

A TUI-launched run executes under a TUI-owned shutdown supervisor that retains its join handle and cancellation token. Orderly TUI closure signals cancellation, waits for active model/host cleanup and child reaping, then releases leases. It does not accept the run. Abrupt TUI/process death closes the command-supervisor liveness pipe; the child supervisor terminates its trusted process group, and OS leases release. Stale `Running` records normalize on the next resume. The `--yes` CLI form is the long-running alternative in its own process.

### OS-locked durable event transaction

Every event writer—decomposition reporter, pause/cancel controls, terminal finalizer, and run-end observer—uses one `WorkflowStore::append_event_transaction` primitive:

1. acquire an OS lock scoped to the run event stream;
2. check/deduplicate the stable event ID;
3. allocate the next sequence under that same lock;
4. serialize the complete record plus newline;
5. append and sync before releasing the lock.

Separate `next_event_seq` and append calls are forbidden. This removes cross-process duplicate/reordered sequences.

### Run-owned progress outbox

Every producer submits typed `DecompositionProgress` to the run-owned reporter:

```text
DecompositionProgress
  event_id
  phase
  subject
  logical_attempt
  kind
  message
  exact_finding?
```

The reporter persists a small outbox record before delivery. It then:

1. commits the durable event through the locked transaction;
2. appends and flushes the matching `.decompose.log` line;
3. marks durable/log delivery complete in the outbox;
4. enqueues transient TUI/CLI delivery.

On crash, outbox recovery uses stable IDs to append whichever durable/log side is missing without duplicating the other. Failure to append/flush `.decompose.log` is operational and prevents the phase checkpoint.

### Log contract

The log path is:

```text
tasks/<PRD>/.decompose.log
```

It is append-only across resume and begins with run ID plus binary/script/catalog identities. Each resume appends a resume marker. It records phase banners, author attempts, model-call-in-flight announcements, host stages, body verdicts, retries, exact findings, and final summary.

It excludes prompts, candidate artifact bodies, environment values, and secrets. One progress event over 64 KiB is operational rather than silently truncated. Stable event IDs make duplicate replay lines identifiable.

The task-root writer lease prevents concurrent decomposition runs from appending this task root's log.

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

- whole chain absent: persist `observer_eligibility = LegacyAbsent`;
- any freeze-chain artifact present: persist `observer_eligibility = Expected`, canonical task-root identity, expected artifact-path set, and the observed portable acceptance identity/digests when readable.

This snapshot is authority for end-of-run observer eligibility. A run that launched `Expected` cannot become legacy-silent by deleting or renaming every artifact; later absence is observer-operational. A run that launched `LegacyAbsent` remains byte-identical legacy behavior even if unrelated artifacts appear later.

`FinalizationRecord` records terminal-state commit, terminal-event commit, and observer intent/state derived from the persisted launch snapshot. `Expected` includes durable `observer_pending` at terminal-state persistence; `LegacyAbsent` omits observer state entirely.

Startup reconciliation and `workflow resume` both recover incomplete finalization. A completed run with pending terminal event or observer is eligible for finalization-only resume; implementation work is never rerun. Before observer checks, the reconciler atomically transitions `observer_pending` to `observer_claimed { owner, lease_generation, claimed_at }` under the run transaction. Only that owner executes. A live claim refuses a second executor; process death/stale lease permits one recovery claim. Stable event/invocation IDs deduplicate records, while the claim prevents duplicate side effects.

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

### Acceptance check execution boundary

Model-authored `AcceptanceCheck::Command` and residual-gap `fail_closed_check` text never reach host `hostCommand` argv, a host shell, or a permissive Bash registry path.

They execute only inside a mandatory ephemeral isolated acceptance world with:

- read-only project/repository/task mounts;
- scratch-only writable tmpfs;
- network disabled;
- empty environment and no provider credentials, host sockets, devices, or daemon endpoints;
- bounded process/memory/CPU/output limits;
- command text delivered over stdin to a shell inside the isolated world;
- teardown by destroying the entire container/VM/world, which contains detached sessions as well as ordinary descendants.

If the configured platform cannot supply and prove that containment, command checks are observer-operational and are not executed. There is no host fallback. Tests run hostile command text that tries to mutate a host sentinel, read ambient secrets, access the network, and leave a detached descendant; all effects must remain absent after world teardown.

`AcceptanceCheck::Floor` splits into pure declarative kernel fields and an optional nested `typed_verifier_command`. Pure floor fields use existing deterministic kernels. Any nested verifier is model-authored command text and executes only in the same mandatory isolated acceptance world; it never reaches the existing host-shell contract verifier. Omitting a frozen nested verifier is forbidden. The synthetic proof may use a floor with no nested command so the observer seam is deterministic.

### Residual-gap semantics

Whenever `acceptance-residual-gaps.json` exists, the observer loads and validates it regardless of whether criteria passed. If absent, the gap set is empty. It applies the existing `validate_residual_gaps` kernel, then requires:

- exactly one record for each failed, `gap_permitted` acceptance ID that claims coverage;
- no record for supplementary, currently passing, unknown, duplicate, or non-permitted IDs; all-pass plus any stale/extra gap is operational;
- every frozen required field present and every forbidden phrase absent;
- the record's `fail_closed_check` passing in the same mandatory isolated acceptance world.

A failed permitted criterion is shadow-covered only by one valid record plus a passing fail-closed check. Failed supplementary or non-permitted criteria cannot be covered. Missing/malformed/extra/mismatched gap data and isolated-world infrastructure failures are observer-operational. Uncovered failed criteria produce run-end policy shadows. Terminal run status remains unchanged in R2.

Run-end command/fail-closed checks have a 300-second timeout and 1 MiB stdout/stderr limits inside the isolated world. Timeout, overflow, launch/teardown failure, malformed residual-gap data, or integrity mismatch is observer-operational.

Run-end shadow records use stable invocation IDs and gate ID `run_end_acceptance`; they include run ID, criterion/gap ID, verbatim finding/remedy, source path, timestamp, and binary revision.

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
- Starting binary revision/hash, run-executable snapshot, and per-spawn same-image identity: execute from the verified handle where supported, otherwise require a supervised pre-capability child handshake proving its loaded image revision/hash; test live path replacement between verification and spawn.
- Binary/script/catalog mixed-identity resume refusal.
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
- Two-phase prepare/parent-commit freeze publication: overflow/nonzero/timeout/crash after child preparation publishes nothing; rollback/recovery covers every multi-file boundary.
- Two-phase body preparation/parent commit and rollback/recovery at every boundary.
- Committed receipt adoption reconstructs the call record without a second judge call.
- Accepted reuse requires exact receipt-produced contract/body, lock, pin, envelope, and stable shadow-record membership digests; later JSONL appends do not invalidate prior receipts and a different valid freeze is not reusable.
- Explicit adopted-predecessor receipt or re-freeze required for pre-existing valid chains; internal consistency alone cannot skip.
- Read-only set-gate identity/postcondition binds the complete PRD/freeze/task/evidence input manifest.
- Task-root writer lease excludes two different run IDs and CAS refuses a stale writer.
- Complete declared write-set confinement and dirfd/no-follow TOCTOU tests cover every child-produced path.
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
- Run lease plus canonical task-root writer lease and stale-running reconciliation.
- TUI shutdown supervisor retains the executor handle, signals cancellation, reaps host work, and releases leases; abrupt parent death triggers supervisor cleanup.
- Happy-path and refusal/requeue durable event sequences.
- One OS-locked event transaction proves stable-ID deduplication and sequence+append atomicity across reporter, lifecycle control, finalizer, and observer processes.
- One OS-locked generation-CAS state/checkpoint transaction prevents lost updates across fanout, pause/cancel, finalization, call records, and resume.
- Observer claim lease prevents two finalizers/reconcilers from executing checks concurrently and recovers a stale claim.
- Author dispatch ledger crash tests cover prepared/in-flight/result-prepared/committed boundaries without fresh budget or duplicate provider work.
- Progress outbox recovers every crash point across durable event → log → transient ordering.
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
- Crashes between each finalization step recover through finalization-only resume without rerunning implementation.
- Failing criterion under global enforce: shadow written, terminal unchanged.
- Isolated acceptance-world absence, timeout, output overflow, teardown failure, and permission denial remain observer-operational with no host-shell fallback.
- Hostile frozen command, nested floor verifier, and fail-closed text cannot mutate a host sentinel, read parent secrets, reach network/daemons, or leave detached descendants after world teardown.
- Residual gaps validate whenever the file exists, including all-pass+stale-gap; bind one authorized record per failed permitted criterion, reject extras/duplicates/non-permitted/supplementary coverage, and execute `fail_closed_check` only in the isolated world.
- Frozen command text never reaches `hostCommand` argv or a host shell.

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
- a frozen acceptance chain;
- one deterministic host-valid but unmet artifact criterion outside task write ownership.

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
10. Prove the unmet criterion creates run-end shadow evidence.
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
- a built-in capability cannot declare and confine its complete write set;
- the current executable cannot be pinned and verified per spawn;
- two run IDs can concurrently write one canonical task root;
- a declared direct command can detach or cannot be bounded/reaped for every reachable child shape on the target platform;
- raw acceptance/fail-closed command text could execute on the host or without the mandatory isolated acceptance world;
- gate commands would be duplicated by a parallel validator;
- an artifact must be published from truncated/malformed output;
- a legacy run consults a freeze prerequisite;
- terminal finalization lacks recoverable state→event→observer ordering;
- run-end observer logic executes before terminal persistence or can alter R2 terminal status;
- a launch-time `LegacyAbsent` run emits any observer output, or a launch-time `Expected` run can erase/mutate its chain and return silently;
- protected trading files would be edited, staged, committed, or implemented;
- a second Cargo process would contend for the repository target directory;
- the synthetic proof does not complete before the trading proof;
- provider reachability or deployed revision/hash cannot be proven;
- bounded retries exhaust on an operational failure.

## R2 completion

R2 is complete only when:

- the full implementation satisfies this spec and receives independent code review;
- primitive, phase, persistence, UI, resume, observer, and legacy tripwire suites pass;
- the synthetic full-lifecycle proof package is complete;
- the trading decomposition-only proof package is complete;
- all gates remain observing by default and run-end authority remains `ObserveOnly`;
- shadow evidence is reviewed and apparent false positives are classified;
- protected trading WIP is unchanged;
- one local R2 commit is built after commit and atomically deployed to matching binaries;
- no push or CI trigger occurred;
- work stops for evidence review before R3, R4 promotion, or any trading implementation run.
