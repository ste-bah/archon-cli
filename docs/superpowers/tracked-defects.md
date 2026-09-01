# Tracked defects

Verified defects that are real, reproduced, and deliberately **not** being fixed
in the change that found them. Each entry carries the evidence needed to act on
it without rediscovering it.

A defect leaves this file only when it is fixed with a test, or when it is
shown not to be a defect — with the reasoning recorded either way.

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

<a name="note"></a>
**Standing test pattern.** For every fix here: write the test red first, then
delete the *call site* while leaving the helper intact and confirm the test
fails. A helper that exists and is never invoked is this codebase's signature
failure mode — it produced both defects above, and the two defects fixed on
2026-08-31 (`4aef5e222`, `6fe31ec4a`).
