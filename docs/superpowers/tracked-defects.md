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

---

<a name="note"></a>
**Standing test pattern.** For every fix here: write the test red first, then
delete the *call site* while leaving the helper intact and confirm the test
fails. A helper that exists and is never invoked is this codebase's signature
failure mode — it produced both defects above, and the two defects fixed on
2026-08-31 (`4aef5e222`, `6fe31ec4a`).
