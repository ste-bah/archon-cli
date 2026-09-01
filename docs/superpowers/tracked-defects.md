# Tracked defects

Verified defects that are real, reproduced, and deliberately **not** being fixed
in the change that found them. Each entry carries the evidence needed to act on
it without rediscovering it.

A defect leaves this file only when it is fixed with a test, or when it is
shown not to be a defect — with the reasoning recorded either way.

---

# Impact analysis — 2026-09-01

Written after a full section-by-section audit of the R2a implementation against
`docs/superpowers/specs/2026-08-27-decomposition-r2-engine-native-design.md`
(spec commit `84b1d8466`). This section exists because the defect list alone
answers the wrong question. The question that matters is *which of these
actually caused the failures we lived through*, and the honest answer is: **most
of them did not.**

## The headline

The engine's core machinery is sound. The publication transaction, call
identity, and resume-identity checks are rigorous (Appendix A). The audit found
**no defect that prevents decomposition or implementation from completing.**

What we actually suffered was three separate things, and conflating them is why
the diagnosis took a week:

1. **An observability blackout** that made every failure unreadable.
2. **One live routing defect** (TD-001) that wasted attempts and requested
   harmful changes.
3. **A regression introduced by the fix for (1)** — TD-011 — which is now the
   most serious item in this file.

## Group A — why the failure was invisible

These do not break the pipeline. They break the ability to *know* it is broken.
This is where the lost week went.

| Defect | Effect |
|---|---|
| **TD-009** | `.decompose.log` records `findings=<count>` and never the text. Nineteen findings, including the one that mattered, were durably recorded as unreadable integers. |
| **TD-006** | `DecompositionPhaseStarted`, `AuthorAttemptRejected`, `ShadowFindingsObserved` never fire; `ModelCallInFlight` does not exist. Retries and rejections are invisible. |
| **TD-010** | `workflow status` omits remaining timeout, budget, and elapsed — the exact fields needed to tell a live run from a stuck one. Answered by grepping proxy logs instead. |

**TD-009 is the single most damaging entry in this file.** Every other defect
here cost hours. This one cost days, repeatedly, because it removed the evidence
needed to find the others.

## Group B — live defects that actually bit

| Defect | Evidence |
|---|---|
| **TD-001** | Reproduced on three separate runs. A finding whose own reducer said *"do NOT route this to either task's remediation"* was routed to two tasks' write-capable remediation worktrees. Requested a change that would have broken a frozen focused test. |
| **TD-002** | The judge recorded a `counterexample` that is the exact opposite of what the contract does, and it validated. |

## Group C — latent holes that have not fired

Real, correctly logged, but with **no evidence** any of them caused an observed
run failure. Recorded so they are fixed before they do.

| Defect | What it needs to bite |
|---|---|
| **TD-003** | A publication the parent refuses — its shadow records survive the rollback. |
| **TD-004** | An out-of-phase or unknown scope, e.g. external mutation during Phase C. |
| **TD-005** | A resume plus a body edit that leaves frozen fields intact. |
| **TD-007 / TD-008** | Parent death or a `setsid` escape. TD-007 *was* observed as orphaned child processes after killing a run, but that is cleanup, not function. |

## The regression — TD-011

The 2026-08-31 change that fixed Group A's root cause **introduced a worse
defect than the one it fixed.** The repair loop now instructs the author to
"fix" `AC-SYN-001`'s commandless floor — a shape the synthetic PRD *mandates*
field by field and a live test asserts.

The plan that drove it
(`~/.claude/plans/iterative-puzzling-parasol.md:113-116`) asserted "the PRD text
merely requires the artifact to exist." The PRD says the opposite. Nobody
re-read it, including two adversarial reviews (Appendix D).

**The synthetic PRD is not defective.** It is the best-specified artifact in this
effort: a deliberately adversarial fixture engineered to produce a criterion no
task may satisfy *and* a non-falsifiable floor, so the run exercises observe-mode
shadow evidence and the run-end observer. It did its job. It caught a real defect
in our code, and we misread the catch as a defect in itself.

## Fix order

1. **The decomp24 verification below** — it decides whether anything else is
   trustworthy.
2. **TD-011** — a regression actively producing wrong behaviour.
3. **TD-001** — the only defect with three reproductions of harmful behaviour.
4. **TD-009** — until the log carries finding text, every future failure costs
   days instead of minutes.
5. **TD-004, TD-005** — they pass bad state *silently*; they do not fail loudly.
6. Everything else.

## The verification that gates all of it

decomp24 reported **19 findings → 0, `Accepted`**, and that was treated as
success. Given TD-011, zero findings implies the `AC-SYN-001` finding
disappeared, which implies the PRD-mandated floor shape was changed.

**Freeze the synthetic fixture and compare the frozen floor against
`synthetic_floor()` in `tests/workflow_decomposition_synthetic_live.rs:16-24`.**

- Identical → decomp24 stands, TD-011 has not yet corrupted output.
- Different → the decomposition is producing PRD-violating contracts, and every
  "green" run since 2026-08-31 was green because the code learned to satisfy the
  gate by abandoning the specification.

Until that check runs, **no run result from 2026-08-31 onward should be treated
as evidence of anything.** The run store for those runs is gone (`find` for
`wf-*` returns only `archive-20260824/workflows-dead-runs`), so this must be
re-derived from a fresh run rather than recovered.

## The pattern behind all of it

Nine of the eleven defects are one shape: **a correct mechanism exists and the
wrong thing consults it.** `routeFindings` computes scopes and drops the
unmatched ones. The events enum declares variants nothing constructs. The
set-gate postcondition re-reads every task file and compares the wrong property.
`append_log` receives findings and writes their count.

This is why test suites stayed green throughout: they exercise the mechanism and
never the call site. See the standing test pattern at the end of this file.


---

## TD-001 — `remediateFindings` routes findings no task may act on

**Status:** open · **Found:** 2026-08-31, run `wf-77813d42` (impl20) ·
**Area:** `crates/archon-workflow/src/v2/script/v3_primitives.js`

The reduce stages emit ownership metadata on every finding. `findingsByTask`
groups solely on task-id fields and reads none of it:

```js
const raw = finding && (finding.canonical_task_ids || finding.task_ids
  || finding.taskIds || (finding.task_id ? [finding.task_id] : []) || []);
```

Occurrences in the whole prelude: `attributable_to_task` **0**, `cross_task`
**0**, `canonical_task_ids` 13.

So a coverage-audit finding carrying

```
attributable_to_task: false
cross_task: true
owner: "none (no canonical task, no workflow stage owns the artifact)"
canonical_task_ids: ["TASK-SYN-010", "TASK-SYN-020"]
```

is routed to **both** tasks' remediation loops, because `canonical_task_ids` was
populated as *context* (the tasks the criterion spans), not as ownership.

**Why it matters.** The reducer's own recommendation on that finding read *"Do
NOT route this to either task's remediation"* — and it was routed anyway, into a
write-capable worktree, at `review-remediate-task-syn-010-1-10` and again at
`review-remediate-task-syn-010-2-12`. The requested "fix" was actively harmful:
satisfying `AC-SYN-001` means creating `.archon/proof/synthetic-observer-target.json`,
which fails `TASK-SYN-020`'s frozen focused test `test ! -e <that path>`.

**Shape of the fix.** Honour `attributable_to_task` / `cross_task` in
`findingsByTask`: a finding marked unattributable goes to `unassigned`, never to
a task group, regardless of `canonical_task_ids`. Test that a finding with
`attributable_to_task: false` produces no per-task remediation call, and sabotage
the call site — see [behavioural tests cannot prove wiring](#note).

---

## TD-002 — the acceptance judge is a rubber stamp

**Status:** open · **Found:** 2026-08-31 from the 2026-08-28 freeze ·
**Area:** `crates/archon-workflow/src/task_set_contract.rs`
(`validate_acceptance_structure`, `require_judgments` branch)

Judgment validation checks only that three strings are non-empty:

```rust
for (field, value) in [
    ("counterexample", criterion.judgment.counterexample.as_str()),
    ("reason",         criterion.judgment.reason.as_str()),
    ("host_call_id",   criterion.judgment.host_call_id.as_str()),
] {
    if value.trim().is_empty() { return invalid(...) }
}
```

Nothing checks the judgment against the contract it judges. On the frozen
synthetic acceptance contract the judge recorded:

```json
"verdict": "accepted",
"counterexample": "No filesystem-state counterexample is constructible: any
  passing state must have .archon/proof/synthetic-observer-target.json parse as
  JSON with boolean ready equal to true."
```

for a floor with `min_instances: 0`, which passes when that artifact is
**absent**. The stated reason is the opposite of what the contract does, and it
validated.

**Why it matters.** A judge that cannot be wrong adds no safety, and its verdict
is what downstream reads as evidence the criterion was examined. Note the
policy layer *did* independently flag the same criterion
(`MissingExecutionObligation`, "floor is not falsifiable") — so the contradiction
between judge verdict and policy finding was available and unused.

**Shape of the fix.** At minimum, a judge verdict of `accepted` must not stand
on a criterion that `acceptance_policy_findings` reports on; the finding should
override the verdict, or the disagreement should be surfaced as its own defect.
Validating judge prose in general is out of reach; validating that it does not
contradict a machine-checkable finding is not.

## TD-003 — shadow records escape the publication transaction

**Status:** open · **Found:** 2026-09-01 during spec audit ·
**Area:** `src/command/workflow_gate_envelope.rs` (`stage_gate_evaluation`),
`src/command/workflow_gate.rs` (`append_shadow_records`, `ShadowRecord`)
**Introduced by:** commit `740c50d25` — my own fix for unreadable freeze findings.

`stage_gate_evaluation` runs in the **child** command. Every caller is a staged
child CLI path: `workflow_staged_cli.rs:45,117`, `workflow_freeze_cli.rs:319`,
`requirement_trace/staged.rs:46`. It calls
`append_shadow_records(cwd, &evaluation.findings, "staged")`, which appends to

```
<cwd>/.archon/logs/workflow-gates-shadow.jsonl        (workflow_gate.rs:196-198)
```

That path is **outside the staging root** and appears in **no declared write
set** (`grep shadow src/command/workflow_host_command_catalog.rs` → nothing).

**Four spec clauses broken.**

1. "The child may only create a `PreparedPublication` … It cannot rename live
   targets or mint a committed receipt." The child commits live bytes here.
2. "The acceptance transaction covers final judged contract bytes, acceptance
   lock, host pin, gate envelope, and **stable shadow records**." Shadow records
   are supposed to be staged and parent-committed.
3. The declared write set is the complete statement of what a child may write,
   and `audit_prepared_publication` enforces `actual == declared == manifest`
   over the **staging tree only** (`workflow_host_command_publish.rs:124`). A
   live write outside that tree is structurally invisible to the audit.
4. §277: the receipt must carry "stable shadow-record IDs plus canonical
   per-record byte digests and locked membership proof". `PublicationReceiptV1`
   (`publication.rs:24-30`) has no shadow field at all.

**Why it matters.** Two concrete failures, not just a layering complaint:

- **Rejected publications leave evidence behind.** The parent refuses a
  publication on non-zero exit, digest mismatch, sentinel violation, or timeout.
  The shadow records were already appended and are never rolled back, so the log
  carries evidence from a call that never committed — against "No partial state
  is a pass."
- **No idempotency.** `ShadowRecord` (`workflow_gate.rs:171-181`) has no call or
  invocation id — only `gate_id`, `finding`, `subject`, `source_path`, `mode`,
  `timestamp`, `binary_commit`. The spec's duplicate-evidence detection on
  one-run resume ("Gate invocations carry a stable invocation ID. Shadow
  records/envelopes include it") has nothing to key on. A retried call appends
  duplicates undetectably.

**Shape of the fix.** Stage shadow records as a declared output like every other
artifact: give each a stable id derived from the call id, write them into
`{COMMAND_STAGING}`, add that path to the capability's declared write set, and
have the parent commit them and record their ids plus per-record digests in
`PublicationReceiptV1`. Never digest the whole mutable JSONL. Test that a
publication the parent **refuses** leaves the live shadow log byte-identical —
and sabotage the call site, per the standing pattern below.

---

## TD-004 — `routeFindings` silently drops unclassified findings

**Status:** open · **Found:** 2026-09-01 during spec audit ·
**Area:** `src/command/workflow_decompose_v1.js:256-275`

The routing loop has four branches and **no final `else`**:

```js
if (scope === "prd_input" || scope === "operational") routed.fatal.push(text);
else if (scope === "inherited_predecessor") routed.inherited.push(text);
else if (retryScopes.has(scope)) routed.retry.push(text);
else if (scope === "body") routed.fatal.push(text);
// (nothing)
```

A finding matching none of these lands only in `routed.all`. Both `routed.retry`
and `routed.fatal` stay empty, so `authorCandidate` takes

```js
if (routed.retry.length === 0) { requireCommitted(outcome, policy.phase); return outcome; }
```

at `:230-233` and **the phase accepts the artifact**.

**Two spec rules broken.**

- Phase C.9: "Mutation of an already accepted body/current frozen chain, or any
  new `Skeleton`/`PrdInput` finding not caused by the candidate, stops as
  operational/external mutation." Phase C retry scopes are
  `{candidate_artifact, body}` (`:131`), so a `skeleton`-scoped finding falls
  through every branch and is swallowed. External mutation of the frozen
  skeleton is accepted silently.
- Candidate publication transaction: "Unknown/missing scope, unknown operational
  kind, or unclassified constructor **is operational**." A missing or
  unrecognised `remediation_scope` is ignored instead of stopping the phase.

The `else if (scope === "body")` arm shows the fall-through was noticed for one
scope and patched only there, instead of making the default fatal.

**Shape of the fix.** Terminal `else` that pushes to `routed.fatal` — unknown,
missing, and out-of-phase scopes are all operational. Test each of: a `skeleton`
finding in Phase C, a finding with `remediation_scope` absent, and a finding
with a garbage scope string. Sabotage by deleting the `else` and confirm each
test fails.

---

## TD-005 — set-gate reuse is blind to task body content

**Status:** open · **Found:** 2026-09-01 during spec audit ·
**Area:** `src/command/workflow_host_command_postcondition.rs:110-124`,
`src/command/workflow_host_command_catalog.rs:165-220`

Phase D.1 requires "one canonical set-gate input manifest: PRD digest,
acceptance/skeleton/lock/pin digests, every sorted task path+digest, and every
evidence/index input declared by the gate", and D.5 requires "Read-only gate call
identity and reuse include the complete manifest digest".

**No such manifest exists** — `grep -ri 'set.gate.*manifest'` returns nothing.

Both set-gate capabilities pass only *paths* (`{TASK_ROOT}`, `{PRD_PATH}`,
`{GATE_ENVELOPE}`, `{CALL_ID}`) with `StdinDelivery::None` and
`max_stdin_bytes = 0`. Since call identity is
`BLAKE3(domain ‖ command_id ‖ catalog_digest ‖ binary_revision ‖ tokens ‖ stdin)`,
**the call id does not vary with task content.**

Reuse is not naively keyed on the id — `record_is_reusable`
(`workflow_host_command_exec.rs:272-303`) also verifies published bytes against
the receipt and re-evaluates the postcondition live. But for set gates the
postcondition falls through to `compare_task_set(&tasks, &skeleton)`, and

- `compare_task_set` (`task_skeleton.rs:351`) compares **task-ID sets only**
  (missing / extra ids);
- `compare_frozen_task` (`:303`) compares **frozen fields** — `task_id`,
  `file_name`, `depends_on`.

Neither digests body content.

**Failure scenario.** Phase D accepts `task-set-lint`. A task body is then edited
— remediation, external mutation, or a manual edit before resume — changing
prose, focused tests, deliverables, or acceptance citations while leaving every
frozen field and the task-id set intact. On resume, call identity is unchanged
(paths only), `receipt_matches_live` passes (the gate envelope on disk is
untouched), and the postcondition reports satisfied. The gate is **skipped and
the stale envelope reused**, so lint and trace never examine the edited bodies —
precisely the content those two gates exist to check.

**Shape of the fix.** Build the D.1 manifest, fold its digest into the resolved
token map so it enters call identity, and have the set-gate postcondition
recompute and compare it. Test: accept a set gate, edit one body leaving frozen
fields intact, and assert the gate re-executes rather than reusing. Sabotage the
manifest token and confirm the test fails.

---

## TD-006 — four typed events are specified but never fire

**Status:** open · **Found:** 2026-09-01 during spec audit ·
**Area:** `crates/archon-workflow/src/events.rs:39-53`

"Meaningful transitions produce both durable and transient representations."
Measured against the spec's fifteen-name list:

| Event | State |
|---|---|
| `ModelCallInFlight` | **absent from the codebase entirely** |
| `DecompositionPhaseStarted` | in the enum, **0 emit sites** |
| `AuthorAttemptRejected` | in the enum, **0 emit sites** |
| `ShadowFindingsObserved` | in the enum, **0 emit sites** |

The other eleven have at least one non-test emit site. `AuthorAttemptCompleted`
exists in the enum and is emitted but is not in the spec's list.

**Why it matters.** These are not cosmetic. `AuthorAttemptRejected` is the only
event that would record a candidate being refused and re-authored — the repair
loop restored on 2026-08-31. That loop is therefore invisible in `.decompose.log`
and in the TUI: a phase that burned five attempts looks identical to one that
succeeded first time. `DecompositionPhaseStarted` never firing is why live run
output jumps straight to `author_attempt_started` with no phase boundary.
`ShadowFindingsObserved` never firing compounds [TD-003](#) — findings are
neither transactional nor announced.

**Shape of the fix.** Emit the three declared variants at their transitions and
add `ModelCallInFlight`, or amend the spec if a name is genuinely unwanted —
but the enum must not keep variants nothing constructs. Assert on the event
stream, not the helper: drive a phase that rejects one candidate and assert
`AuthorAttemptRejected` appears in `events.jsonl`, then delete the emit call and
confirm the test fails.

---

## TD-007 — the supervisor has no liveness pipe, so parent death is undetected

**Status:** open · **Found:** 2026-09-01 during spec audit ·
**Area:** `src/command/workflow_host_command_supervisor.rs:92-95`, `:166`

The permission-and-process contract requires the parent to start the supervisor
"with a **liveness pipe**", step 3 to race completion "against timeout,
persisted pause/cancel, and **parent-pipe closure**", and step 4 to terminate the
process group on "**parent death**".

`supervise_process_group` takes `(ResolvedHostCommand, HostCommandControl)` and
nothing else; its only caller (`workflow_host_command_exec.rs:69`) passes no
parent handle. The `tokio::select!` at `:166` has four branches — child wait,
control, supervisor event, timeout. There is no fifth.
`grep -riE 'liveness|parent_pipe' src/command/` matches only unrelated
subsystems (worktree ownership, stage board).

**Failure scenario.** The archon parent is killed or crashes while a host command
is running. The child process group is never signalled and survives, holding its
staging root and any open handles. Observed in practice: killing a run left child
processes that had to be hunted separately.

**Shape of the fix.** Pass the read end of a pipe held open by the parent into
the supervisor, add a `select!` branch on its closure, and route it to the same
`terminate_and_reap` path as timeout. Test by dropping the parent handle
mid-command and asserting the group is reaped.

---

## TD-008 — termination never audits for surviving descendants

**Status:** open · **Found:** 2026-09-01 during spec audit ·
**Area:** `src/command/workflow_host_command_supervisor.rs:308-326`

Step 6 of the supervisor contract requires it to "audit that no known descendant
remains". `terminate_and_reap` signals the group with SIGTERM, sleeps
`CLEANUP_GRACE`, signals SIGKILL, then reaps **the direct child** with
`child.wait()` under `REAP_DEADLINE`. Nothing enumerates or re-checks
descendants afterwards, and `descendant` appears nowhere in the file. The
function returns `Ok(())` regardless.

`signal_group_members` (`:345`) is not this: it is a fallback for when the group
kill itself fails, not a post-condition audit. Killing a group and verifying
nothing survived it are different obligations; the spec requires both.

**Failure scenario.** A child calls `setsid`, leaving the process group. SIGTERM
and SIGKILL to `-pgid` never reach it, `child.wait()` reaps only the direct
child, and the supervisor reports clean termination while the escaped process
keeps running — potentially still writing into the staging root the parent is
about to audit and publish.

**Shape of the fix.** After reaping, enumerate surviving processes for the group
and fail the call if any remain. Test with a command that deliberately
`setsid`s a child and assert termination reports the survivor rather than
`Ok(())`.

---

## TD-009 — `.decompose.log` records finding counts, never finding text

**Status:** open · **Found:** 2026-09-01 during spec audit ·
**Area:** `src/command/workflow_decompose_state.rs:60,338-352`

The log contract requires the log to record "phase banners, author attempts,
model-call-in-flight announcements, host stages, body verdicts, retries, **exact
findings**, and final summary."

`append_log` emits exactly one line shape:

```
event_id={seq} phase={phase} {subject_key}={value} attempt={} disposition={} findings={} status={} reused={}
```

`findings=` is `projection.finding_count` — an integer. No finding text is
written anywhere in the module; `finding_count` is the only findings-related
field that exists.

**This is the root of the 2026-08-24..31 failure.** The durable record of the
synthetic decomposition read

```
event_id=7   acceptance   accepted_with_shadow_findings  findings=1  needs_review
event_id=15  skeleton     accepted_with_shadow_findings  findings=3  needs_review
```

Nineteen findings existed, including the non-falsifiable `AC-SYN-001` floor, and
the only durable evidence said *how many*. A week was spent building on a task
set whose defects were recorded but unreadable.

Combined with [TD-006](#), the log is missing four of the contract's eight
elements: phase banners (`DecompositionPhaseStarted` never emitted),
model-call-in-flight announcements (event does not exist), retries
(`AuthorAttemptRejected` never emitted), and exact findings.

**Note on the 2026-08-31 fix.** Commit `740c50d25` routed freeze findings to the
shadow JSONL, which made the text reachable *somewhere* — but not in
`.decompose.log`, which is the file the spec names and the file an operator
reads. It also introduced [TD-003](#).

**Shape of the fix.** Carry the exact finding text on the projection and write
one log line per finding, subject to the existing 64 KiB operational bound. Test
that a phase with two policy findings produces both texts in
`tasks/<PRD>/.decompose.log`, and sabotage by reverting to the count.

---

## TD-010 — `workflow status` omits seven required fields

**Status:** open · **Found:** 2026-09-01 during spec audit ·
**Area:** `src/command/workflow_decompose_status.rs`

Status detail is specified as fourteen items. Present and correct: run
kind/template version, starting binary revision, script/catalog digests, current
phase, active host capability id, unresolved shadow count, last operational
error, `.decompose.log` path, resume-eligible calls, per-subject attempts and
dispositions, provider route origin.

Missing:

| Required | State |
|---|---|
| remaining active timeout | absent |
| attempt budget | logical attempt shown, budget not |
| active model | provider route origin only, no model |
| elapsed time | absent |
| accepted/interrupted/failed call counts | counts are by *method* (authors/bodies/host_commands), not by status (`:105-118`) |
| body totals: pending, accepted, accepted-with-shadows | per-subject labels listed, never aggregated |
| finalization/observer state | absent |

**Why it matters.** Remaining timeout, budget, and elapsed time are exactly the
fields needed to answer "is this run alive or stuck", which recurred throughout
the 2026-08 proof runs and had to be answered by grepping proxy logs instead.

**Shape of the fix.** Aggregate call records by status rather than method, total
body dispositions, and surface deadline/budget/elapsed from the persisted attempt
state. Assert each field appears for a run in a known state.

---

## TD-011 — the repair loop demands the author violate the PRD

**Status:** open · **Found:** 2026-09-01 · **Severity: highest — this is a
regression I introduced on 2026-08-31.**
**Area:** `src/command/workflow_task_set.rs:193`,
`src/command/workflow_decompose_v1.js:85`

The synthetic PRD (`tests/fixtures/decomposition-synthetic/prd.md`) does not
merely require an artifact to exist. `AC-SYN-001` **dictates the freeze shape
field by field**:

> Freeze this criterion as a commandless floor with `kind=...`,
> `artifact_path=...`, `artifact_format="json"`, `required_true_fields=["ready"]`,
> **every other floor field at its serde default, and no
> `typed_verifier_command`**.

`tests/workflow_decomposition_synthetic_live.rs:16-24` asserts exactly that
shape via `..Default::default()`. The commandless floor is the **correct**
output, not a defect.

That mandated shape triggers the finding deterministically: with no command,
`verifier_strength_defect` (`verifier_strength.rs:53-56`) returns
`MissingExecutionObligation` unless the contract has a positive instance
obligation, and `min_instances` at serde default is 0. **The fixture is
engineered to produce this finding** — along with a criterion no task may
satisfy — so the run exercises observe-mode shadow evidence and the run-end
observer.

**The defect.** `acceptance_policy_findings` are mapped to
`RemediationScope::CandidateArtifact` (`workflow_task_set.rs:193`), and
`candidate_artifact` is in acceptance's `retryScopes` (`:85`). Since the
2026-08-31 removal of the observe-mode early return, that finding is fed back to
the author as a defect to repair. The author is being told to fix a floor the
PRD mandates and a test asserts.

Both outcomes are wrong: obey the PRD and burn all six attempts before falling
back to `bestCommitted`, or satisfy the gate by freezing a contract that
violates the PRD.

**The plan that caused it was wrong on the facts.**
`~/.claude/plans/iterative-puzzling-parasol.md:113-116` states "the author chose
a commandless floor with `min_instances: 0`; the PRD text merely requires the
artifact to exist." The PRD says the opposite. The repair loop was built on that
misreading and reviewed without anyone re-reading the PRD.

**Unverified consequence.** decomp24 reported 19 findings → 0, `Accepted`. Zero
findings implies the floor finding disappeared, which implies the shape changed
and the frozen contract no longer matches the PRD or `synthetic_floor()`. The
run store is gone, so this is an inference. **Verify before trusting decomp24 or
anything built on it.**

**What was actually right.** The deleted early return produced
`accepted_with_shadow_findings` for `AC-SYN-001` — the correct disposition. The
general repair loop is still needed (skeleton and body findings genuinely
required feedback); the error was sweeping a PRD-mandated shadow finding into
the repair path with them.

**Shape of the fix.** A policy finding about a contract shape the PRD explicitly
mandates is not `candidate_artifact`. Either give PRD-mandated shapes their own
non-retrying scope that records a shadow and continues, or have the acceptance
phase recognise that a finding it cannot clear without contradicting the PRD is
terminal-observe, not repairable. Test: run the synthetic fixture and assert the
frozen floor still equals `synthetic_floor()` **and** the run records
`accepted_with_shadow_findings` rather than re-authoring. That test is the real
acceptance criterion for the 2026-08-31 change and it was never written.

---

# Appendix A — verified correct, do not "fix"

Recorded so that a future pass does not "repair" working code, and so the
defect list is not mistaken for a verdict on the whole engine. Each was checked
against the spec during the 2026-09-01 audit.

| Area | Finding |
|---|---|
| **Host-computed call identity** (`host_command.rs:48-75`) | **Better than spec.** Every component is length-framed, so `(a,bc)` and `(ab,c)` cannot collide. Spec only requires concatenation. |
| **Publication transaction** (`workflow_host_command_publish.rs`) | Sentinels verified twice (`:117`, `:194`); staged tree must exactly equal declared write set *and* manifest (`:124`); duplicates rejected; symlinks refused (`:150`, `:333`); per-entry length and digest checked; destination set equality; prior digests captured; **post-commit re-read and digest comparison** (`:230-238`). All four spec demands met. |
| **Two-phase wiring** | `PreparedPublicationV1` / `PublicationReceiptV1` are genuinely wired end to end: envelope produces (`workflow_gate_envelope.rs:83`) → exec reads (`exec.rs:388`) → decision gates (`decision.rs:22`) → publish mints (`publish.rs:250`) → postcondition consumes (`postcondition.rs:138`). Not orphan types. |
| **Resume identity** (`workflow_decompose_resume.rs:85-140`) | Strongest code audited. Script source compared **byte-for-byte** against the embedded script, plus template version, binary revision, catalog, PRD digest, canonical launch arguments, project root, provider route — each with a named remedy. |
| **Reuse safety** (`exec.rs:272-303`) | Not naively keyed on call id: re-verifies published bytes against the receipt and **re-evaluates the postcondition live**. (Its weakness is *what* the set-gate postcondition compares — TD-005 — not the mechanism.) |
| **Environment policy** (`supervisor.rs:100-101`) | `.env_clear()` then explicit `.envs()`. `CommandCapability` carries only an `EnvironmentProfileId`; `ResolvedHostCommand` holds values but has **no `Serialize` derive**; the single tracing call logs pid and error only. |
| **Input/output limits** | All five capabilities match the spec table. Stdin overflow rejects **before spawn** (`catalog.rs:356`); drains are concurrent; limit crossing terminates and returns `Err`, so truncated output cannot be published. |
| **Tool policy** (`workflow_live_v2_client.rs:125-134`) | Missing policy errors; **empty allowlist rejected** exactly as spec requires; `EXACT_TOOL_POLICY_MARKER` correctly suppresses `ALWAYS_ALLOWED` (`subagent_executor.rs:292`); `write_roots` empty; no Bash/Write/Edit. |
| **Attempt budgets** (`workflow_decompose_v1.js:74-76`) | 6 / 6 / 10 — exactly the spec's "six logical attempts, body authoring ten". 1,500 s backstop wired. |
| **`stopReason` discipline** (`:201`) | Only `end_turn` accepted, so truncated and max-token outcomes are never parsed as partial JSON. |
| **Finding routing shape** (`:256-275`) | `operational_error` throws; `prd_input`/`operational` fatal in every mode; `inherited_predecessor` non-retrying and non-blocking; per-phase retry scopes match the spec exactly. (TD-004 is the missing `else`, not the shape.) |
| **64 KiB progress bound** (`workflow_decompose_state.rs:66-70`) | Returns `StageFailed` — operational, **not** silent truncation, exactly as specified. |
| **Event emission order** (`:72-83`) | Durable event → `.decompose.log` → transient UI, with `seq` as the stable id and `sanitize_value` applied first. Matches the spec's required order. |
| **Log header / resume marker** (`workflow_decompose_log.rs:52-83`) | Run id plus binary/script/catalog digests, field-sanitized against whitespace/control/`=`, written at launch and on every resume, via symlink-safe `append_nofollow_line`. |
| **Dry-run** (`dry_run_b.rs:27-48`) | Stub carries every field the fixed script reads, marks `dryRun: true`, never spawns. |
| **No `continue` alias** | Confirmed absent from the production CLI surface. |
| **The synthetic PRD** | Not defective. See TD-011 — it is a correctly-built adversarial fixture and it caught a real defect. |

---

# Appendix B — candidates chased and dropped

Recorded so they are not "rediscovered" and logged as defects by a later pass.

**`StdinDelivery::AtomicOverlay` does not exist.** The spec names it for body
candidates. `land-task-body` (`catalog.rs:129-160`) instead uses
`StdinDelivery::Utf8Bytes` with `--candidate-stdin --staging-root`, and its
declared write set is `{COMMAND_STAGING}/{FROZEN_TASK_FILE_NAME}` +
`{GATE_ENVELOPE}` — staging only, never the live path. The property the spec
requires (child prepares, parent commits, live bytes untouched) **holds**. Enum
variant naming only. **Not a defect.**

**No launch-time tool-policy pre-flight.** The spec wants the launcher to fail
"before the first author dispatch if a required name is absent after registry
filtering". The check instead runs at dispatch (`client.rs:125-134`), and names
are not intersected with a registry, so nothing is silently dropped today. The
one latent drop path is `DENYLIST` (`subagent_executor.rs:280`). **Minor
divergence, no demonstrable failure — logged here rather than as a defect.**

**`freeze-skeleton` gets `EnvironmentProfileId::None`** where the spec says
freeze capabilities receive a named provider profile (`catalog.rs:107`).
**Stricter than spec and correct** — skeleton freeze makes no model call.

**`OutputLimitExceeded` naming.** Chased against the spec's error-class list and
dismissed: that list enumerates *classes*, not typed names.

---

# Appendix C — audit scope and method

**Scope.** Every section of the spec was read and checked against source:
input/output limits, environment policy, permission/process policy, call
identity and produced-output binding, candidate publication transaction,
provider-neutral authoring, Phases 0 and A–E, resume model, progress/TUI/CLI and
the `.decompose.log` contract, typed event vocabulary, dry-run semantics,
authority pin, and R2a deferrals. **Nothing was left unaudited.**

**Method.** Every claim is tied to `file:line`. Where a type or helper existed,
its *call sites* were checked separately — the codebase's signature failure is a
correct helper nothing invokes. Two candidate findings were withdrawn on
evidence rather than banked (Appendix B).

**Limits on the evidence.** Run-state claims (decomp24, impl31) rest on
observations made at the time, **not** on anything re-read during the audit: the
run store for those runs is gone. Anything depending on them is marked as an
inference with a named verification step, never as fact.

---

# Appendix D — why review did not catch this

TD-011 passed a written plan, an adversarial review by a second party, and a
self-review, then shipped. The failure mode is worth recording because it will
recur otherwise.

- **Nobody re-read the PRD.** The plan asserted what the PRD required
  (`:113-116`); every subsequent reviewer took that assertion as the premise and
  reviewed the reasoning built on it. The assertion was false and was never
  checked against the file.
- **A "correct in general" fix was applied to a case where it was wrong.**
  Feeding findings back to the author was genuinely needed for skeleton and body
  findings. A PRD-mandated shadow finding was swept in with them.
- **Green tests confirmed the mechanism, not the outcome.** The acceptance
  criterion for the change — *the frozen floor still equals `synthetic_floor()`
  and the run records `accepted_with_shadow_findings` rather than re-authoring*
  — was never written as a test. Had it existed, it would have failed
  immediately.
- **"0 findings" was read as success.** For this fixture it is closer to a
  failure signal, because the fixture is built to produce one.

**Standing rule:** before treating a finding as repairable, read the PRD text
that governs it. A finding the author cannot clear without contradicting the PRD
is a legitimate observe-mode shadow, not a defect to fix.

---

<a name="note"></a>
**Standing test pattern.** For every fix here: write the test red first, then
delete the *call site* while leaving the helper intact and confirm the test
fails. A helper that exists and is never invoked is this codebase's signature
failure mode — it produced nine of the eleven defects above, and the two fixed
on 2026-08-31 (`4aef5e222`, `6fe31ec4a`).

For TD-011 specifically the pattern is different and needs its own rule: the
code was wired correctly and did exactly what it was told. The specification it
was told to follow had been misread. **Check the fix against the governing
document, not only against the code.**
