# Tracked defects

Verified defects that are real, reproduced, and deliberately **not** being fixed
in the change that found them. Each entry carries the evidence needed to act on
it without rediscovering it.

A defect leaves this file only when it is fixed with a test, or when it is
shown not to be a defect — with the reasoning recorded either way.

---

# Fix status — 2026-09-01

Every defect below has a fix in the working tree, pending the verification run.
Two items are only partly closed and say so in their entry:

| Defect | Fix |
|---|---|
| TD-001 | `findingsByTask` reads `attributable_to_task` / `cross_task` before ids |
| TD-002 | an `accepted` verdict on a criterion the policy layer reports is now itself a finding |
| TD-003 | shadow records moved out of the staged child to the parent's post-commit path, carrying the call id |
| TD-004 | terminal `else` in `routeFindings`; set gates given an explicit shadow scope |
| TD-005 | set-gate input manifest digest folded into call identity |
| TD-006 | **partial** — `AuthorAttemptRejected` and `ShadowFindingsObserved` now emit; `DecompositionPhaseStarted` and `ModelCallInFlight` still do not |
| TD-007 | `ProcessGroupGuard` kills the group if the supervisor is dropped; full parent-death detection still needs a child-held pipe |
| TD-008 | `audit_no_descendants` after every termination path |
| TD-009 | one log line per finding, carrying subject, scope and exact text |
| TD-010 | **partial** — call counts by status and subject totals added; timeout/budget/elapsed/model still absent |
| TD-011 | acceptance policy findings no longer routed as retryable candidate defects |
| TD-013 | **fixed** — the finding rules (arrays, identity, attribution) have one owner, `v2::review_findings`; the host attaches the set and the accounting check is host-against-host; a build-time guard forbids a prelude copy |
| TD-014 | **fixed** — `assert_observer_after_terminal` selects the terminal event by its `terminal_status` marker, not an event kind nothing emits (`84aed38d3`) |
| TD-015 | **fixed** — acceptance-policy findings are `CandidateArtifact` (repaired) unless the PRD criterion prescribes the check shape in the engine's contract vocabulary (`criterion_prescribes_check_shape`); proof 2 had frozen all 11 trading ACs unfalsifiable with no repair |

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

**Fixed 2026-09-01, after a live run showed the mechanism is worse than logged.**

The first fix keyed on `attributable_to_task` / `cross_task`. The adversarial
review pointed out that **nothing in the system emits either field**, so the
check never fired; a live run confirmed it.

The real mechanism, from run `wf-cdeb4fa4` (2h39m, failed at the last stage):

```
Error: agent() targetFiles must list at least one literal repo-relative file path
       for write work
    at assertPathList (eval_script:318)
    at agent (eval_script:340)
    at remediateFindings (eval_script:1050)
```

`remediateFindings` dispatched a **write-capable** agent for
`ac-syn-001-absent-by-design` — a finding whose own reviewer wrote *"still open
at PRD level, not closable by a task"*. No task owns a file that would satisfy
it, so `targetFilesFor` returned nothing, `agent({write: true})` threw, the
script died without returning, and the host reported `authored workflow returned
no task accounting` — an error three layers removed from the cause.

So this defect does not merely waste remediation attempts. **It kills the run at
the final stage**, after decomposition, implementation, verification and both
reviews have already succeeded.

**The fix uses the signal that actually exists.** A finding that yields no
writable target cannot be remediated by that task, whatever metadata it carries.
`remediateFindings` now records it as `outcome: "not_task_actionable"` with a
reason and dispatches nothing, so it stays visible in the accounting — which is
what the host checks — instead of crashing. The `attributable_to_task` check is
kept as a cheap upstream guard for the day a reducer does emit it.

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

## TD-012 — a healthy implementation run's terminal status is not stable

**Status:** open · **Found:** 2026-09-02, raised in review · **Area:** run
finalization / `RunStatus` for `authored_task_workflow`

Two healthy runs of the same fixture, same binary family, same inputs:

| Run | Status | Branches | Blocking gaps |
|---|---|---|---|
| `wf-56746c18` | `NeedsReview` | 13 accepted | 0 |
| `wf-ccf305b8` | `Completed` | 12 accepted | 0 |

Both implemented both tasks, verified them, ran both reviews and reconciled the
accounting. The difference is only whether a reviewer happened to leave an
unresolved finding, since run-end unmet acceptance criteria are observe-only
under the `ObserveOnly` pin and do not move the terminal status.

This is legitimate under the current design, which is why the proof asserts
"not a failure" plus the substantive invariants rather than a fixed status --
the real check lives in `assert_observer_after_terminal`.

**Why it is still a defect.** A terminal status that varies with reviewer whim
carries no information for an operator or for automation: `Completed` and
`NeedsReview` do not distinguish two different outcomes here. Either the status
should be derived from something stable (unresolved findings that a task could
act on, which is a different set from "any unresolved finding"), or the two
statuses should be collapsed for this run kind and the review state reported
separately.

**Shape of the fix.** Decide what the status is *for*, then make it a function
of that. Until then the proof cannot assert it, which is the position we are in.

---

## TD-013 — the finding rules were written four times, and drifted

**Fixed 2026-09-03.** Full account in `finding-identity-duplication.md`
("Resolution"). Six live failures reduced to one shape: the host's walk,
identity and containment in Rust, mirrored by hand in the prelude's JavaScript
(and, it turned out, a third Rust copy in `lifecycle_policy/adversarial.rs`
and a fourth in the offline replay), with nothing in the build that failed on
drift. Resolution: the host computes each review's finding set from the
records it holds and attaches it to the call result; the prelude reads the
attachment; the accounting check compares the script's report with the host's
own attachment. `the_prelude_carries_no_copy_of_the_finding_rules` fails the
build on any regrowth.

A seventh divergence surfaced during the fix: the prelude's attribution table
was keyed by an `item_id` the host never used to name branches, so it matched
nothing live. Attribution now comes from the branch input the host built.

## TD-014 — the observer assertion filtered on an event kind nothing emits

**Fixed 2026-09-03** (`84aed38d3`). `assert_observer_after_terminal` looked
for `kind == "completed"`; the finalizer encodes the outcome in the kind and
stamps `detail.event == "terminal_status"` on every path. Run 22 was the first
run to reach the assertion, so it had never executed. Selection is now by the
marker; two tests replay run 22's recorded events, and restoring the kind
filter fails both.

---

## TD-015 — acceptance-policy findings are never repaired on a real PRD

**Fixed 2026-09-03** (`task_set_contract_policy::criterion_prescribes_check_shape`; the freeze path routes to `CandidateArtifact` unless the criterion prescribes its shape; four tests, including a drift guard that every vocabulary token is a real contract field). **Found by proof package 2, 2026-09-03** (run `wf-26fee43e`, trading PRD,
decomposition only). All 11 acceptance criteria froze as non-falsifiable floors
(a self-reported evidence JSON with boolean flags, `min_instances: 0`), each
refuted by the host judge -- 22 findings -- and none reached the author, because
`workflow_task_set.rs` stamps every acceptance-policy finding
`RemediationScope::InheritedPredecessor` (the TD-011 fix, `77fd82885`) on the
premise that "the PRD may mandate that shape exactly". It does for the synthetic
fixture, whose AC row prescribes a commandless floor. It does not for the trading
PRD, whose AC rows are plain outcome statements backed by commands (§8.7) and
focused tests (§13). The repair loop demonstrably works one phase later (the
skeleton cleared seven findings in three attempts; a body cleared its frozen-field
drift in two) -- this phase simply never asks.

**Consequence.** A contract of checks that cannot fail would let the run-end
observer pass every criterion vacuously: the tautological-verifier disease at
the contract layer.

**Shape of the fix.** Route acceptance-policy findings to `CandidateArtifact`
(repairable) unless the PRD criterion text itself prescribes the check's shape
in the engine's own floor vocabulary; only then are they inherited/recorded. Both
fixtures stay honest: the synthetic proof keeps its byte-identical mandated
floor, and a real PRD gets a falsifiable contract or an exhausted budget that
says so. Red first with this run's `.decompose.log` as the fixture.

## TD-016 — the proof workspace inherits no `[context]` compaction model

**Open, found 2026-09-03** (proof package 1 re-run). The synthetic workspace copies
only `[api]` and `[models]` from the project config, so request-pressure
compaction ran without `compaction_model` and every compaction recorded
`outcome=auto_failed`. Harmless for the proof (the run still completed) but it
means the proof never exercises the compaction path the real project uses.

## TD-017 — subagent turn budget is effectively unbounded

**Open, found 2026-09-03.** `DEFAULT_MAX_TURNS` equals the hard cap (100_000), so
a verification branch has no turn ceiling; one ran for about an hour under the
14400 s timeout. The timeout is the only bound. A per-call turn budget that the
script sets, with the cap as the ceiling, is the fix.

## TD-018 — a refused TUI `/workflow decompose` is visible only on screen

**Open, found 2026-09-03** (two proof-2 launches lost). The refusal reasons
(`refuse_active_task_root`, stale pin) are painted to the TUI and never written
to the session log or the task root, so an operator driving the TUI from a
script sees only the harness timing out. Log the refusal; have the external
harness preflight ownership of the target root.

## TD-019 — a `needs_review` decomposition holds its task root indefinitely

**Open, found 2026-09-03.** Only `Completed` and `Failed` release
`task_root_identity`; a run that ended `needs_review` still owns the root, every
new launch is refused (TD-018 hides why), and `workflow cancel` is policy-denied
outside the owning interactive session. The operator's only path is moving the
run directory by hand. Terminal `needs_review` needs an operator release path.

## TD-020 — the repair author never sees why the judge refuted a check

**Fixed 2026-09-03** (`task_set_contract_policy::refuted_check_message`; judge
prompt in `workflow_task_set_judge.rs`; three tests). **Found by proof package
2, 2026-09-03** (run `wf-6bd69fe0`, the first run with TD-015 in place). The
TD-015 fix did its job: all 22 acceptance findings arrived as
`candidate_artifact`, the author was re-asked, and attempt 2 cut them to 10.
The 10 survivors were all "refuted by the host judge", and two things made that
loop unwinnable:

1. The judge records a `reason` and a `counterexample` per criterion in the
   frozen contract, but the finding text -- the only thing `authorPrompt` carries
   -- said just "refuted; replace the check with one the judge cannot falsify".
   The author repaired blind.
2. The judge prompt asked for "a filesystem state where the check passes while
   the criterion is false" with no boundary. Its counterexamples stubbed the
   program under test on PATH. Under that rule every command-kind check is
   refutable, so convergence was luck.

**Shape of the fix.** The finding now carries the judge's reason and
counterexample, quoted (so the author reads them as evidence, not instruction),
flattened to one line and capped at 400 characters (the repair prompt repeats
every earlier attempt's findings verbatim). The judge prompt fixes the toolchain
-- shell, OS, environment, PATH, every executable the repository does not itself
build -- as out of bounds, and lets everything the implementation produces vary:
the repository's own source, the program built from it, and every file under the
project root. A counterexample that stubs, wraps or shadows an executable must
not refute. The hostile review caught the first draft making "the program under
test" itself out of bounds, which would have forbidden the legitimate
counterexample "the implementation hardcodes the expected output".

**Residual.** The boundary is prompt-only: a judge that ignores it still yields a
refuted verdict the author cannot satisfy. Host-side classification of an
out-of-bounds counterexample is the follow-up if a run shows the judge ignoring
the rule.

**Verified 2026-09-03 23:43** (run `wf-2ac2a1d9` on `299b4d8d4`, the PRD from
proof package 2). Acceptance converged 22 → 12 → 0 on attempt 4 with every
refutation carrying an in-bounds counterexample; 11 command checks, all
judge-accepted; skeleton 20 → 0; 15 bodies accepted with 0 findings; lint and
requirements-trace clean; terminal `completed`. First clean decomposition of a
real PRD.

## TD-021 — the external proof harness waits a fixed 1,500 s for the skeleton phase

**Fixed 2026-09-04** (`tests/support/workflow_decomposition_proof_progress.rs`:
`wait_for_event_line_while_progressing` restarts its idle clock on every new
durable event, fails at once if the run ends without the awaited event, and
keeps a 4 h cap; both live harnesses use it for the skeleton boundary and the
external harness ends on the same idle model instead of a fixed 7,200 s; the
`terminal_status` label is trusted only for genuinely terminal statuses, since
the engine also stamps it on `paused` and `running`; four tests). Residual: the
120 s wait for the interrupted attempt assumes a pause aborts the in-flight
provider call, and the 1,500 s idle window is also the longest single provider
call the harness tolerates, because the engine emits no heartbeat inside one. **Found 2026-09-03** (run `wf-2ac2a1d9`). `wait_for_event_line(...,
["author_attempt_started", "skeleton-author-1"], Duration::from_secs(1_500))`
in `tests/workflow_decomposition_external_prd_live.rs` assumes acceptance freezes
in under 25 minutes. A real PRD's acceptance repair loop costs ~10 minutes per
attempt (author + freeze + batched judge) and the engine allows several attempts,
so the harness failed while the run it was observing went on to complete
cleanly. The evidence package for that run is therefore the run directory and
task set copied by hand, not a harness manifest. Size the wait to the engine's
attempt budget, or wait on observed progress rather than a fixed clock.

## TD-022 — a false artifact claim ends a write branch under review instead of being re-asked

**Fixed 2026-09-04** (`agent_repair::WorkflowV2AgentError::DeclaredArtifactAbsent`;
`agent_adapter_a::validate_request_specific_result` raises it for every
`missing_project_artifact_*` gap that normalization added; two tests).
**Found 2026-09-04** (proof package 1 re-run on `2d57d8629`, run `wf-f368fca4`).
A remediation branch's result declared, as an artifact, the engine's own patch
manifest path for that stage -- a path pattern it had seen in the previous
attempt's result -- and never wrote it. `note_missing_project_artifact` did
its job and recorded "it does not exist" as a *blocking* residual gap; the
branch went `needs_review`, the fanout ended `call_needs_review`, nothing
downstream re-asks a branch, and the run ended with the gap unresolved. The
proof harness refused it (`synthetic_evidence.rs:105`). Two malformed replies
earlier in the same run were repaired by the bounded schema-repair loop; a
false report had no such path. Same engine passed the proof twice before: the
model simply had not made that particular claim.

**Shape of the fix.** A declared path is a claim the host verifies. In a
write-capable result that says the work is done, a claim the host cannot find
is the agent's to repair: the adapter returns `DeclaredArtifactAbsent` carrying
every absent path, which the existing bounded repair loop re-asks about
(contract class, so it shares the budget with other validation failures and
earns a fresh attempt after a malformed reply). On exhaustion it becomes a
stage failure like a schema failure, which the script-level retry already
recovers. Three boundaries, each from two rounds of hostile review:
`normalize_project_artifact_files` now returns the absent claims verbatim as
the agent wrote them (no inference from gaps, so neither a pre-seeded gap nor a
`./`-spelled path can dodge, and an artifact the host added from the repository
fallback, which exists by construction, never counts); the
`missing_project_artifact_*` gap namespace is the host's, so agent-authored
gaps there are dropped before the host looks; and an honest `blocked`, `failed`
or `cancelled` result, judged by the status the agent itself returned before
any host step rewrote it, keeps its gap instead of spending repair budget.
The live host's own repair loop shares `differs_from`, and its host-level
re-ask set deliberately excludes this error, as it does every error that has
already spent its bounded re-ask. Read-only results are untouched. The two
write tests that pinned the old needs_review-with-gap outcome now pin the
error. No path, PRD, or provider knowledge involved.

## TD-023 — one omitted field, one early closer, or one empty reply sinks a whole branch

**Fixed 2026-09-04** (`v2/agent_output_tolerance.rs`; `EmptyReply`; five
tests). **Found 2026-09-04** by reading every rejected reply from the two
failed proof-1 re-runs (`wf-f368fca4`, `wf-b8bccb91`) instead of calling them
model variance. Of six rejections: three were the host refusing a complete,
sensible reply over one field it could have supplied itself
(`residual_gaps[].id` twice, `artifacts[].path` twice); one was a `]` written
where the open gap object still owed its `}`, with every byte of content
present; two were empty replies from the provider that were counted as the
agent's malformed output and burned its only same-class repair. The repair
budget then did exactly what it was told -- one re-ask per class -- and the
branch failed. Each failure cost a 3.5 h proof.

**Shape of the fix.** Host decides, deterministically, with one reading each:
a residual gap without an id gets one minted from its description; an
artifact without a path leaves the evidence list and becomes a visible note; a
closer written early is completed with the one the document demands, while a
reply that stops mid-value is still refused (content may be missing -- the
existing "fails loudly" tests stand; a fenced reply gets the same
completion); an empty reply is `EmptyReply`, an execution-class error whose
"repair" is the original ask again, so it never shares a budget with the
agent's own mistakes, and the generated-PRD reducer stand-in treats it as
repairable like any unanswered reducer. Two boundaries from the hostile
review: the host's note for a path-less artifact is added only beside the
agent's own evidence, never as the only entry (the host must not satisfy its
own evidence gate), and the note names the artifact's description before its
minted id. Same doctrine as the trailing comma repair already in
`agent_output_normalize`: the most common mistakes should not cost a stage.

## TD-024 — the synthetic proof failed a run for a read-only branch it had already recovered

**Fixed 2026-09-04** (`tests/support/workflow_decomposition_synthetic_evidence.rs`).
**Found 2026-09-04** (run `wf-b8bccb91`). The rule "no write branch may fail
or block" read the run's aggregate branch counts, which include read-only
critic branches and give no credit for recovery; a coverage-audit branch that
failed on the TD-023 closer defect, whose call the script re-ran and accepted,
failed the proof. The rule now refuses any `blocked` branch, any announced
(read-only, `stage_failed` with a branch id) failure whose call was not
accepted at call level afterwards -- a sibling branch's acceptance carries the
same call id and no longer counts, nor does an acceptance that precedes the
failure -- and any `failed` count the announced failures do not explain, which
is a write branch that failed and was never re-run (write branches announce
nothing; the review caught that the first draft would have passed them). The
blocking-gap rule uses the same later-call-level-acceptance test.

## TD-025 — the freeze command refuses a candidate over one stray comma and says only "line 3 column 1265"

**Fixed 2026-09-04** (`archon_workflow::json_document::{repair_json_document,
describe_json_fault}`; `workflow_freeze_candidate::candidate_document`; five
tests). **Found 2026-09-04** (proof package 2 run `wf-74a8b262`, under the
harness). Acceptance attempts 4, 5 and 6 each returned a complete artifact
(4.8k to 23.7k chars, `end_turn`), and the freeze refused all three as "not a
JSON document (expected `,` or `]` at line 3 column 1265)" -- one syntax slip
inside an embedded check command, no excerpt, no hint. The author was told a
column number in a 23,000-character line, could not act on it, and the phase
exhausted its budget; observe mode fell back to the best contract seen (11
refuted checks) and the run built a skeleton on it. The agent envelope parser
had had trailing-comma repair, the `<HERE>` fault window and the unterminated
and control-character hints since a111839f3 and TD-023; the freeze path, which
stages every decomposition artifact, had none of them.

**Shape of the fix.** One place owns the host's JSON tolerance and offers it to
every consumer: `repair_json_document` (trailing comma, then a closer written
early; never a truncation) and `describe_json_fault` (serde's message, the
bytes at fault marked, the hint). The freeze stages the repaired bytes when
exactly one reading repairs them, and a refusal now carries the marked excerpt
so the author's next attempt is aimed. PRD- and provider-agnostic.

## TD-026 — resume re-judges an already-frozen contract, and the judge does not agree with itself

**Fixed 2026-09-05** (`archon_workflow::v2::script::history_replay`
(`replayable_history`, `superseded`, `call_family`);
`WorkflowScriptHost::replay_superseded_history`; four tests). A superseded
record -- one a later record of the same *subject* has followed, neither
invalidated: for an agent call the same call-id family at a higher ordinal,
for a host command the same command over the same reported task ids at a
later start -- is answered from the record on a resumed fixed run, whatever
its status, provided the call arrives with the input it was recorded with. The
last record of each subject keeps its live check, and that check is the right
one: every byte its receipt published is still on disk. Run `wf-327f789a`
showed why the executor's own test is not: it refuses any result carrying
findings, so the last acceptance freeze -- the artifact on disk, with the 8
findings the judge had given it -- was re-executed and re-judged to 2, and the
run proceeded on a contract that had never been accepted. The executor now
answers a second question, `record_is_live`: the same live checks (identity,
receipt matching disk, postcondition holding now, subject terminal) with
findings allowed; the last landing of a subject replays verbatim while it is
live, and an operator edit during the pause breaks that and falls through to
live execution (a receipt alone was not enough: an empty receipt matches
trivially, and the existing reject-reuse test caught the first draft). The
hostile review caught
the first draft twice: hooking the executor's reuse check, which the script
host never reaches for a `needs_review` freeze, and keying host supersession
by command alone, which would have let one task's later landing retire
another task's only landing without a live check. **Found 2026-09-04** (run `wf-74a8b262`). After the harness's pause and
`resume --live`, the run replayed the acceptance phase: author attempts 1 and 2
were reused (`reused=true`), but their freezes were re-executed because
`HostCommandResult::reusable` refuses any result carrying policy findings, and
in observe mode the best contract seen legitimately carries findings. The
re-executed freeze re-ran the batched judge on byte-identical content and
returned 21 refutations where the first pass had returned 11. `bestCommitted`
is per script execution, so the pre-pause best (11) was lost and the resumed run
settled on a worse contract (22). Two defects: a replay must reuse the recorded
freeze of an identical candidate instead of asking a non-deterministic judge
again, and the best-of across a pause must survive the pause. Real
decompositions do not pause, so this bites the proof harness's pause/resume leg
first; it is still an engine defect.

**Where it lives.** `workflow_host_command_exec::record_is_reusable` is a
*live-state* test by design: the recorded receipt must match what is on disk
now, the postcondition must hold now, the subject must be terminal now. That is
the right question for "is the artifact already the result of this command",
and the wrong question for replaying history: every freeze before the last one
in a phase is superseded on disk by construction, so it can never pass, and the
replay re-executes it. Confirmed on `wf-aed51b7a` (ff7a0ce8a): after resume,
attempts 1 and 2 were `reused=true` for the author and re-judged for the freeze
(22 findings again for attempt 1). **Shape of the fix (proposed, not built):**
on replay, a host-command record whose outcome was superseded by a later record
of the same phase is history and is returned verbatim from the record, keyed
only by call identity and input hash; the live-state test applies to the
phase's final record alone. `bestCommitted` then carries across the pause for
free, because the replayed outcomes are the recorded ones.

**Confirmed fatal under the harness, 2026-09-05 03:48** (`wf-aed51b7a`,
ff7a0ce8a). Before the pause the acceptance phase converged at attempt 5 with
0 findings. After `resume --live` the replay reused the author replies of
attempts 1 and 2 (checkpointed as accepted) and re-executed everything else:
the freezes were re-judged (22, then 11), the author calls of attempts 3 to 6
were re-run because their recorded results were `needs_review` (a refused
candidate), the new replies drew four fresh refusals, the six-attempt budget
was spent a second time, and observe mode fell back to the best contract of
the *replay* (11 refuted checks). The accepted contract was on disk as the
attempt-5 record and was never consulted. Two rules are therefore needed, not
one: a superseded host-command record replays verbatim (above), and a
superseded *agent* record replays verbatim too, whatever its status, because a
refused or malformed attempt is history exactly as much as an accepted one.
"Superseded" = a later record of the same phase exists from before the resume;
the phase's last record keeps today's live checks, so a run resumed after a
crash still re-runs the call that was in flight.

## TD-027 — a candidate the host could not read is charged to the author's budget

**Fixed 2026-09-05** (`workflow_decompose_v1.js`: `PACKAGING_REFUNDS`;
`archon_workflow::json_document::repair_local_slips`; two tests, verified
against both refused documents from the run). **Found
2026-09-05** (run `wf-327f789a`). Four of six acceptance attempts were spent on
candidates the freeze refused before judging anything: two quote slips inside
embedded shell commands (a bare `"` in `'"$p"'`, and `\""` where the escape
was meant as the close), one missing field, one gap-policy mismatch. The phase
exhausted its budget at 8 findings and the run built on that. Two answers, both
PRD- and provider-agnostic: the host repairs local slips itself, each where
the parser trips, one at a time, validated by the parse that must succeed at
the end -- an element opening where a key was expected (the previous one was
never closed), an array closed while an object is open, closers left over after
the root, and a bare quote with content rather than structure after it; the
second refused document turned out to be nine entries never closed with the
closers piled at the end, not a quote slip at all -- and a refusal the host
could not even parse is refunded to the candidate budget, bounded to three per
phase, so packaging never masquerades as authorship while a model that can
never package a document still stops.

## TD-028 — an author told to fix one check rewrites the whole contract, and the host let it

**Fixed 2026-09-05** (`workflow_task_set_merge::keep_previously_accepted`;
three tests; the kept entry must also agree with the candidate's gap
declaration, and the diagnostic goes to stderr because stdout is the manifest
the host parses). **Found 2026-09-05** (run `wf-435b00a4`, the first run with
TD-026 and TD-027 in place: zero JSON refusals, every attempt judged). The
acceptance phase went 22, 1, 22, 1, 11 findings and ended one check short:
each time the author was handed a single refutation it re-authored every
criterion, regressing checks the judge had already accepted, and the budget
was spent alternating. The prompt's history of earlier attempts did not stop
it. The host now keeps what it has accepted: after judging, a criterion the
new candidate gets wrong (refuted, or carrying a host policy finding) takes
the entry and stored verdict of the same id from the freeze on disk when that
entry was accepted and clean, describes the same PRD, and carries the same
criterion text. Progress is monotone per criterion, the judge is never asked
twice about one check, and the freeze reports which ids it kept. In the run
above, attempts 2 and 4 would have merged to zero findings at attempt 4.

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

## TD-029 — the acceptance author prompt read as "send the example as shown"

**Live-verified 2026-09-05 20:42, run `wf-c040450a` (binary 192045760)** (`workflow_decompose_v1.js` acceptance prompt; the
missing-checks refusal in `task_set_contract.rs`). **Found 2026-09-05** (run
`wf-42e9bf31`, the first run with TD-028 in place). Attempts 1 and 2 of the
acceptance phase each returned the shape example itself: one entry, empty
criterion, placeholders intact, stop reason `end_turn`, 530 and 560 bytes
against 5,282 bytes for the identical prompt (same input hash) one run
earlier. The model had read the PRD (its one entry named a real artifact
path) and still sent one entry. The prompt showed a single entry and then
said "send the placeholders shown"; the refusal said "add exactly one check
per PRD acceptance id", and the repair attempt answered with exactly one
check. Both readings are the text's fault, not the model's. The prompt now
says the two entries show alternative shapes and not the count, that
the artifact carries one entry for every acceptance id the PRD defines, and
that fewer entries than ids is refused; the host-overwritten fields are named
as the only ones to send as placeholders. The refusal now says to keep every
check present and add one for each listed id. Two of six attempts were lost
before the third returned all eleven checks; nothing here knows a PRD.

The same run showed the second half of the defect: the shape example offered
only a floor with `required_true_fields`, so every check the author sent was
a presence assertion on an artifact the implementation itself writes, and
the judge refuted all eleven with the same reason (a stale or hand-placed
file passes while the deliverable fails). The prompt now shows both check
shapes, `floor` and `command`, and states the judge's standard: a check must
fail in every state where its criterion is false, a presence-only floor is
refuted, and a command that runs the deliverable and exits non-zero in that
state is the falsifiable shape. Command checks are deferred at run end in
R2a by design; the judge still holds them to the standard.


## TD-030 — mechanically weak acceptance floors consumed judge calls

**Live-verified 2026-09-05 20:42, run `wf-c040450a`: zero preflight bounces in the passing run; runs 10 and 11 each bounced two mechanical defects in about two minutes per round.** Positive instance
counts/source bindings are legitimate task inventory floors, but not evidence
that an acceptance criterion's deliverable executes correctly. Acceptance policy
now requires an executable verifier for floors. `workflow_acceptance_preflight.rs`
runs mechanical checks before `judge_contract`, refusing candidate-owned defects
with `CandidateRejected` (the staged CLI emits `candidate_artifact`). Explicitly
PRD-prescribed shapes retain the existing inherited-observation policy; this does
not promote observe gates to enforce. Deterministic acceptance refusals use a
separate bounded repair allowance (six repairs per judged-attempt budget slot),
never one of the six judged-candidate slots; exhausted repairs stop explicitly.
Tests: production freeze boundary (zero judge calls, unchanged disk), positive
counts and source bindings, full JS author loop budget/refusal termination.

Review (Claude, 2026-09-05 18:05): the mechanical allowance was `attempts * 6`
(36 author calls, hours at minutes per call) and it swallowed packaging
refusals, which also start with the refusal prefix, so a model that never
packages a document got 36 calls instead of TD-027's 3. Now a flat
`ACCEPTANCE_REFUSAL_REFUNDS = 12`, packaging tested first and kept on its own
bound. The endless-refusal test expects 12. Six `trading_data` tests fail in
the full binary suite; they read Steven's uncommitted `crates/archon-trading`
work and are unrelated to this change. One pipeline timing test
(`compilation_timeout_terminates_descendant_when_direct_child_exited`) fails
under full-suite load and passes alone; unrelated.

## TD-031 — acceptance judge silently used default sampling

**Live-verified 2026-09-05 20:42, run `wf-c040450a`: every frozen judgment records temperature 0, the resolved model and the provider.** A sampled completion
method crosses the workflow port, pipeline adapter and subagent fallback to the
provider. Judge requests use temperature 0; each frozen judgment records requested
temperature, resolved model and provider. Ordinary calls keep their defaults.
Messages and OpenAI-compatible transports forward temperature; transports without
explicit support fail operationally instead of silently dropping it. No model or
operator config is changed. Tests drive the judge through the production adapter
chain and inspect serialized Messages requests, including default omission.
Live run 10 (`wf-8b3957f5`, binary fccb5045d, 2026-09-05 18:35): the first
candidate to pass preflight reached the judge and the run died with "provider
does not support explicit temperature". The runtime wraps every provider in
`ObservedLlmProvider` (`src/runtime/provider_observer.rs`), and the new trait
method's default is `false`; the wrapper did not forward it, so the transport's
answer never reached the adapter. The mocked tests exercised bare transports
only. The observer and `CodexAutoProvider` now delegate; a test wraps a real
Anthropic transport and a fake and asserts each answer passes through.
Temperature 0 reduces sampling variation but does NOT guarantee identical remote
verdicts. Identical-contract/two-run reproducibility and convergence are live proof
criteria still outstanding, not established by mocked provider tests.

## TD-032 — invented acceptance-check fields were silently discarded

**Live 2026-09-05: no invented field appeared in runs 10 to 12; the deterministic tests stand.** `AcceptanceCheck`
and its shared deliverable-floor type now reject unknown fields through serde.
The existing staged refusal path names the offending field and routes it to the
author before judging. The shared floor type moved unchanged to a small module
apart from strict deserialization; valid documented fields retain their defaults.
Tests pass unknown fields at both check and nested-floor levels through real
freeze preparation and assert candidate rejection, exact field name and zero
judge calls. Existing published contracts without sampling provenance still parse.

### Acceptance-loop release verification boundary

No proof 2 is launched by this change. Two consecutive live acceptance phases
within six judged attempts, identical verdicts for identical contracts, and the
live stderr retention message remain for the operator's proof. A freeze-boundary
regression exercises the existing TD-028 retention call with accepted then refuted
responses; that is not a claim of live model convergence. Cargo is barred until
`ps -Ao comm | grep -c '^\./archon'` reports zero. Commit precedes compilation.


### TD-029–032 deterministic verification (2026-09-05)

- `cargo check --bins --tests`: exit 0 (existing warnings remain).
- Acceptance/freeze root tests: 43 passed; author loop: 10 passed; operational
  author budgets: 2 passed; run-end observer: 7 passed.
- Workflow library: 1304 passed, 4 ignored. Contract/verifier integration: 11
  passed, 1 ignored. Pipeline library: 736 passed. Wire/provider integration:
  7 passed. These are scoped suites, not a claim that the entire workspace is green.
- Sabotage build a96f53e13 disabled the preflight refusal, strict check decoding,
  sampled judge dispatch, and freeze retention call. All four targeted tests
  failed (exit 101) as required. Restored in 79cc8843a. Author budget test was
  also observed failing on the old JS loop, then passing with refunds.
- Retention emitted `kept previously accepted checks for AC-X-001` in the
  freeze-boundary test. This confirms the real call site, not live convergence.
- Sampling is carried to the wire and stored in judgment records. Unsupported
  sampling transports refuse operationally. Remote repeatability is NOT proven;
  no proof-2 run was launched. The operator still owns both live runs.

## TD-033 — the judge's adversary was unbounded, so every check was refutable

**Live-verified 2026-09-05 20:42, run `wf-c040450a`: judged attempt 1 accepted 5 of 11 with honest refutations, attempt 2 accepted 11 of 11, and the run ended Completed (acceptance clean, skeleton clean at 15 tasks on attempt 3, 15 bodies, pause/resume replayed 4 calls, harness green in 4,839 s). Fixed 2026-09-05** (`workflow_task_set_judge.rs` rubric; author standard in
`workflow_decompose_v1.js`; judge prompt test). **Found 2026-09-05** (run
`wf-125bd7f1`, binary bfabeaef2, the first run where a candidate reached a
temperature-0 judge). The author sent eleven command checks that build and run
the deliverable and run named repository tests. The judge refuted all eleven,
and its reasons were of one kind: "the test is repository source and can be
trivially written", "a program that always answers true passes the check".
The rubric fixed the toolchain (an earlier fix, for a judge that stubbed
executables) but let the implementation vary without limit, so the judge was
free to assume an implementation written to game the check. Under that
assumption no check can ever pass: a check cannot tell a correct implementation
from one crafted to satisfy it. At temperature 0 the same contract draws the
same eleven refutations every time, so the phase could never converge.

The rubric now bounds the adversary: the implementation is fallible, not
adversarial. It may be missing, partial, wrong, stale, empty, malformed or
hand-placed, its tests may be absent or narrower than the criterion, its output
may be an error message; it does not write source, tests or data whose purpose
is to satisfy the check. A counterexample that needs such deliberate gaming is
invalid and must not refute. Honest refutations survive: a named test that does
not exist (a zero-match filter exits 0), a file with the right fields and the
wrong content, a substring match on an error message. The author prompt states
the same standard so the two sides judge by one rule. Nothing here knows a PRD.

## TD-034 (open) — mechanical refusals outside the acceptance phase still cost a judged attempt

**Found 2026-09-05** (run `wf-c040450a`, the passing run). Skeleton attempt 1
was refused by the host for a non-canonical task id, a defect the host decided
without a judge, and it cost one of six skeleton attempts because the TD-030
allowance in `workflow_decompose_v1.js` is gated on `policy.phase ===
"acceptance"`. The same rule should hold for every phase: a refusal the host
decided deterministically names its defect and is not a judged attempt. Not
fixed; the run converged anyway.

## TD-035 (fixed) — Native nested command success bypassed declarative prerequisites

**Found 2026-09-06**, native acceptance branch. The scratch adapter now evaluates the existing floor kernel before the nested verifier. Missing artifacts fail rather than being replaced by a zero shell exit.

**Regression/evidence:** `workflow_run_end_native_tests::native_nested_verifier_cannot_pass_when_its_floor_is_missing`.

## TD-036 (fixed) — Native observation selected launch revision instead of final implementation revision

**Found 2026-09-06**, native acceptance branch. The finalizer records the final repository commit once before terminal publication; recovery retains it after HEAD moves.

**Regression/evidence:** `native_final_source_records_implementation_commit_not_launch_commit`.

## TD-037 (fixed) — Unused native configuration broke commandless observation

**Found 2026-09-06**, native acceptance branch. Commandless contracts use the existing pure observer without requiring a native profile.

**Regression/evidence:** `native_policy_on_commandless_contract_keeps_existing_floor_evaluation`.

## TD-038 (fixed) — Native dispatch lacked a persisted terminal identity guard

**Found 2026-09-06**, native acceptance branch. Native composition checks finalization flags, terminal status and the persisted snapshot before policy parsing or dispatch. This does not replace the separately gated crash protocol.

**Regression/evidence:** `native_dispatch_refuses_before_terminal_persistence`.

## TD-039 (fixed) — Direct credential-file selection bypassed recursive copy filtering

**Found 2026-09-06**, native acceptance branch. Copy validation checks the selected path itself as well as descendants. This is filename filtering of operator-declared inputs, not secret-content classification.

**Regression/evidence:** `directly_selected_credential_file_is_not_exported`.

## TD-040 (fixed) — Native observations could overlap on one repository

**Found 2026-09-06**, native acceptance branch. The guardian owns a nonblocking OS lease until teardown, including after parent death. The lease is tested both in-process and against a competing subprocess before and after release.

**Regression/evidence:** `native_execution_lock_rejects_overlapping_observations`.

## TD-041 (fixed) — Fast commands escaped the periodic scratch quota check

**Found 2026-09-06**, native acceptance branch. Scratch size is checked again after command exit and process teardown.

**Regression/evidence:** `short_command_cannot_escape_scratch_size_check_by_exiting`.

## TD-042 (fixed) — Changed tracked source could feed a later warm-target check

**Found 2026-09-06**, native acceptance branch. Tracked source manifests are compared after every executed command; differences stop the observation. Native completion additionally captures tool binary digests, effective build environment, target/cwd-link object identity, Cargo configuration and per-check cache/data inventories.

**Regression/evidence:** `changed_scratch_source_cannot_feed_a_later_check`.

## TD-043 (fixed) — Committed Cargo configuration was rejected during source construction

**Found 2026-09-06**, native acceptance branch. Recorded source copy preserves committed Cargo configuration; declared project copies still reject host configuration.

**Regression/evidence:** `combined_view_preserves_committed_cargo_configuration`.

## TD-044 (fixed) — Standard registry metadata made a Cargo seed unusable

**Found 2026-09-06**, native acceptance branch. Only operator-selected registry/git subtrees are copied. Their package metadata is retained; Cargo-home credentials/config are not exported. The seed must be operator-verified credential-free.

**Regression/evidence:** `cargo_seed_keeps_registry_metadata_without_exporting_home_credentials`.

## TD-045 (fixed) — Native evidence omitted selected command and copied-input provenance

**Found 2026-09-06**, native acceptance branch. The observation record includes command references, cwd mappings, effective policy, initial copied-project inventory and cleanup error. Per-check build identities, Cargo cache digests and changed project paths are now recorded; shared scratch data is explicit (`input_reset=false`).

**Regression/evidence:** `evidence_binds_commands_policy_and_copied_inputs`.

## TD-046 (open) — Residual execution has no pinned judged command binding

**Found 2026-09-06**, native acceptance branch. ResidualGapRecord contains fail_closed_check text but no lock/judge binding. Task 9 forbids inventing new freeze semantics. Native authorization refuses this shape; valid residual execution, coverage and routing sabotage cannot be claimed complete.

**Regression/evidence:** `acceptance_world_authorization rejects unbound residual references`.

## TD-047 (fixed) — Advanced floor prerequisites deferred in native observation

**Found 2026-09-06**, native acceptance branch. The pure floor kernel defers advanced predicates. Native execution now renders the existing host-owned verifier with the authorized floor and its typed command removed, executes those prerequisites through the same bounded scratch process, then executes the original pinned typed command bytes. No new evaluator or unbounded subprocess path is introduced.

**Regression/evidence:** `advanced_floor_runs_shared_verifier_before_exact_nested_command`; dispatch sabotage rejects the invalid artifact when wired, and falsely passes when bypassed..

## TD-048 (fixed) — Native lifetime, failure evidence and cache verification gaps

**Found 2026-09-06**, native acceptance branch. Setup/copy/inventory phases now check cancellation and phase deadlines. Git checkout has bounded drain/reap and a process group. Guardian request delivery and total wait are bounded. Setup and after-audit failures retain nonpassing records. Worktree removal is verified and failures retained. Target replacement and changed build configuration invalidate reuse; per-check identities/cache digests/project deltas are recorded. Filesystem calls cooperate between operations; this is not guaranteed interruption of an uninterruptible kernel syscall or confinement of deliberately detached descendants.

**Regression/evidence:** `acceptance_scratch_completion` (nine focused cases), `guardian_partial_request_cannot_wait_forever`, existing parent-SIGKILL regression and advanced/identity/setup call-site sabotage..

## TD-049 (fixed) — Live quota scan mistook removed temporary files for observation failure

**Found 2026-09-06.** The advanced interpreter removed a scratch temp file between directory enumeration and stat. Only NotFound during recursive child scanning is now ignored; other errors remain operational and the final post-exit size check remains mandatory. Root removal and missing final root still fail.

**Regression/evidence:** `advanced_floor_runs_shared_verifier_before_exact_nested_command` failed with `scratch size audit failed: No such file or directory` before this change; the native suites exercise the corrected scan.

## TD-050 (fixed) — Whole-root acceptance audit was impractical and coupled to workflow output

**Review 2026-09-06:** Replaced whole-root recursion with recorded-commit source paths, declared project inputs minus exclusions, and task-root manifests. Untracked build/VCS/workflow output is outside the audit. Streaming hashes bound memory; nonregular input objects are not followed.

**Regression:** `audit_ignores_untracked_build_and_concurrent_workflow_output`.

## TD-051 (fixed) — Scratch quota walks ran at control-poll frequency

**Review 2026-09-06:** Kept the 25ms cancellation/output checks, moved recursive quota scans to a five-second cadence after each completed scan, and retained final post-exit scan. Walk count is recorded.

**Regression:** `quota_walks_are_coarse_while_cancellation_stays_responsive`.

## TD-052 (fixed) — Earlier checks contaminated later checks through shared project data

**Review 2026-09-06:** Captured an original scratch project baseline and restored it before each check, removing added/changed files. Unchanged source files retain mtimes and Cargo target remains shared. Evidence records input_reset=true.

**Regression:** `later_check_cannot_use_earlier_input_mutations; existing real native build/warm-target test`.

## TD-053 (fixed) — Declared data configuration names were rejected as host secrets

**Review 2026-09-06:** Nested config.json/config.toml and credential-like filenames in operator-approved data are permitted. Root host credential/config selections still refuse; project_input_excludes applies equally to copying and audit. Cargo cache credential filtering remains separate.

**Regression:** `nested_data_configuration_is_not_a_host_secret; operator_exclusions_apply_to_copy_and_audit`.

## TD-054 (fixed) — Voided native observations lost run-owned evidence

**Review 2026-09-06:** Copy available guardian evidence into observer/native-observation.json on failure, or persist a minimal operational record when execution never produced one. Reuse the shared project-root resolver.

**Regression:** `voided_native_observation_retains_run_evidence`.

## TD-055 (fixed) — An ordinary timeout skipped all later independent checks

**Review 2026-09-06:** Continue after per-check timeout/output failure when build/project integrity remains valid and teardown is verified; reset inputs before the next check. Cancellation, identity/audit damage and unverified process cleanup still stop.

**Regression:** `ordinary_timeout_does_not_skip_independent_later_check`.

## TD-056 (fixed) — Native release observation exceeded fixed cleanup budget

**Found 2026-09-06**, first authorized pre-implementation observation at `3eef71178`.
All eleven checks executed (three passed, eight criterion failures), with no per-check
operational errors and unchanged audited inputs. Final teardown nevertheless failed:
`native observation phase deadline exceeded`. `ScratchRoots::cleanup` imposed a fixed
five-second limit on deleting the release target, regardless of the host profile.
The observation remains void; eventual Drop cleanup does not retroactively pass it.

Cleanup now uses the profile timeout with a five-second minimum, and the guardian's
post-cancellation grace permits that same bounded cleanup. The regression runs real
worktree removal through a private Git wrapper delaying removal six seconds: red
under the five-second constant, green under the twenty-second test profile.

**Regression:** `acceptance_scratch_cleanup_budget::cleanup_uses_profile_budget_for_slow_owned_tree_removal`.

## TD-057 (fixed 2026-09-07) — a write agent has no turn bound; its wall-clock bound discards the work

**Found 2026-09-06** (R3 run `wf-f0efefa6`, wave 1, `implement-task-dl-001`). The
subagent runner's turn cap is `SubagentRequest::DEFAULT_MAX_TURNS = MAX_TURNS_HARD_CAP
= 100_000` (`crates/archon-tools/src/subagent_request.rs:55`), by design: "runaway-loop
protection is the USD budget cap, not an arbitrary turn count". A local model through
litellm has no USD cost, so nothing bounds the call. The authored script's `agents()`
write calls set no `timeoutSecs`; the adapter applies a timeout only when the request
carries one (`archon-pipeline/src/subagent_adapter.rs:347`). Observed: one read-only
audit task ran for over 90 minutes, each turn re-sending about 73k prompt tokens and
generating 2 to 4k, one turn per minute, with nothing written to its worktree. The run
cannot end this on its own; only the operator can. Fix after the run, PRD-agnostic: a
default per-call wall-clock budget for workflow agent calls from host config
(`[workflow.generated]`), applied by the adapter when the script sets none, recorded in
the call record, and a turn-count ceiling that scales with the task's declared file
count. Not fixed during the run.

Correction 2026-09-07 04:55: a write branch does have a wall-clock bound. TASK-DL-002
(`agents-2-0`) was ended by the engine after roughly six hours with "write branch timed
out before returning usable output" (`crates/archon-workflow/src/v2/write/errors.rs:200`).
The transcript shows 319 turns, 414 tool calls, 35 cargo invocations, seven writes and
edits across `crates/archon-trading/tests/registry_v2.rs`, `src/command/trading_data/ingest.rs`
and `src/command/trading_data.rs`, and it was still fixing `cargo check` errors when
cut. The turn count is unbounded; the time bound is far longer than a task should take
on this model. See TD-058 for what happens to the work.

## TD-058 (fixed 2026-09-07) — a timed-out write branch discards its partial work and later waves build without it

**Found 2026-09-07** (R3 run `wf-f0efefa6`, wave 2). When `agents-2-0` timed out, its
worktree was removed and no patch was staged under
`write-coordination/stages/agents-2/`; six hours of code are gone. The item recorded
`needs_review` with `files_changed: None`, nothing was committed, and the script moved
to wave 3, whose tasks declare `TASK-DL-002` as a dependency and now run against a
baseline that lacks it. The later per-task remediation pass will re-run TASK-DL-002 from
scratch. Required: on branch timeout, stage the item's current diff as a patch (the
sidecar machinery exists) and hand it to the remediation prompt as the starting point;
mark dependents of a failed wave item as blocked-on-dependency rather than dispatching
them against a baseline that cannot satisfy them. PRD-agnostic.

### TD-057 / TD-058 fix (2026-09-07 07:05)

- `v2/write/partial_work.rs`: when a branch ends without a manifest and without
  acceptance, `collect_worktree_wave_artifacts` captures its worktree diff
  (tracked edits and new files, ignored paths excluded) to
  `write-coordination/stages/<call>/partial/<item>.patch`, records it on the
  branch outcome under `data.partial_work`, and `cleanup_completed_worktree_wave`
  now retains that worktree as a failed workspace. `prepare_worktree_wave` looks
  up the newest partial for the branch's canonical task ids, applies it with
  `git apply --3way` onto the fresh worktree, and the branch runner prepends a
  resume note to the agent's task. Tests: `partial_work_tests` (3).
- `v2/write/dependency_gate.rs`: before a wave is prepared, each assignment's
  canonical tasks are checked against `dependency_ids`; a dependency in the
  universe with no accepted or no-op branch outcome in the run holds the branch
  back with a typed `blocked_on_dependency_<item>` outcome, saved like any
  branch outcome, no worktree, no agent. Tests: `dependency_gate_tests` (2).
- `[workflow.generated] write_call_time_budget_secs` (default 0 = derived
  `3 × host_call_timeout_secs`; validated 0 or 300..=86400) overrides the
  branch call budget in `LiveAgentDispatch`. Test: `live_agent_dispatch::budget_tests`.
- Coverage gap, stated: the three call sites are unit-tested through their
  helpers and read-verified; no wave-level integration test drives a timed-out
  branch end to end, because the crate has no fanout harness. Next live run is
  that test; its evidence goes here.
- `run_one_worktree_branch` moved unchanged from `worktree_branch_a.rs` (519
  lines) to `worktree_branch_run.rs`; `prepare_worktree_wave` moved to
  `worktree_wave_prepare.rs`. Both parents now under the cap.
