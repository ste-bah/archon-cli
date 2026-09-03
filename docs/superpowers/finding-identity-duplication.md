# The finding-extraction rule is written twice

**Not fixed. Handed over for review.** Six defects in 24 hours all reduce to
this one shape, and I patched each instance rather than the cause.

## The duplication

Two independent implementations of "which fields hold findings, and when are two
findings the same".

**Host, Rust** — `crates/archon-workflow/src/v2/script/v3_author_checks_b.rs`

| Function | Line | Responsibility |
|---|---|---|
| `extract_review_findings_from_record` | 307 | entry point over a call record |
| `collect_findings_arrays` | 318 | the walk: unions `findings`, `adversarial_findings`, `uncovered_requirements`; recurses `data`, `result`, then `outcomes` **else** `items` |
| `finding_identities` | 365 | identity fields: `id`, `title`, `claim`, `summary`, `finding`, `requirement_id`, trimmed, first 200 chars |
| `finding_key` | 388 | joins identities, falls back to whole-document JSON |
| `assert_multiset_contains` | 396 | containment by identity overlap, exact multiset for anonymous findings |

**Prelude, JavaScript** — `crates/archon-workflow/src/v2/script/v3_primitives.js`

| Function | Line | Responsibility |
|---|---|---|
| `reviewFindings` | 220 | the same walk, hand-mirrored |
| `findingsFrom` | 250 | delegates to it |
| `attributedMapFindings` | 286 | per-branch walk plus attribution |
| `findingIdentities` | 308 | the same six identity fields, hand-mirrored |
| `mergeMapAndReduceFindings` | 344 | drops a reduce finding sharing ANY identity with a map finding |

The host checks the prelude's output against its own walk of the same records.
Any divergence is reported as the script hiding or inventing findings, after a
run that otherwise succeeded — typically 2.5 hours in.

## The six divergences, all live failures

| # | Divergence | Symptom | Fixed in |
|---|---|---|---|
| 1 | Prelude read `data.findings` only; host unioned three keys | accounting a subset; `expected 2 found 0` | `b8d8ae5ed` |
| 2 | Host walked `items` **and** `outcomes`; prelude walked `outcomes` | host demanded a second copy; `expected 2 found 1` | `df73b13e4` |
| 3 | Host keyed on exact JSON; prelude enriches with `canonical_task_ids` | enriched finding reported missing | `59f90368f` |
| 4 | Host demanded every reduce finding; prelude drops restatements by design | `expected 1 found 0` on a finding deliberately dropped | `bdbf92783` |
| 5 | `Object.assign({}, "AC-SYN-001", …)` shredded a bare-string finding | coverage finding lost as `{"0":"A",…}` | `bdbf92783` |
| 6 | Host walks `outcomes` **else** `items`; prelude still walked both | accounting carried a finding the host never visited | `1a4c097d3` |

Each fix corrected one side. The next run found the gap the fix had opened on
the other. #2 and #6 are the same rule, changed on the host and not mirrored,
four commits apart.

## Why patching does not converge

The rule has to hold across a language boundary with no shared type, no shared
test, and no mechanism that fails when the two drift. Nothing in the build
notices; only a live run does, at ~2.5 hours a probe.

The offline replay at `/private/tmp/archon-replay/replay.mjs` re-implements the
host walk a **third** time to catch these in under a second. It works — it found
#4 and #5 — but it is another copy of the same rule, so it can drift too. It
missed #6 until I added the reverse containment check, because it only tested
`accounting ⊇ reviewers`.

## Options, for review

1. **Host computes the accounting.** The prelude stops extracting; the host
   walks the records it already has and hands the script the finding set. One
   implementation, and the check becomes a tautology — arguably it should not
   exist at all.
2. **Prelude calls the host.** Expose the walk as a host function on `w` and
   have `reviewFindings` delegate. One implementation, script-side control
   retained.
3. **Keep both, add a drift test.** A fixture corpus of recorded records,
   asserted to produce byte-identical output from both. Cheapest, and the only
   one that keeps a third copy honest — but it still permits drift between
   releases of the corpus.

My reading: (1) removes the class. The check exists because the script is
trusted to report what the reviewers said; if the host builds that set from
records it already holds, there is nothing for the script to get wrong.

## Evidence

Preserved runs under `/private/tmp/archon-r2a-workspaces/synthetic-*` — real map
and reduce records for every failure above. `replay.mjs` reconciles both
containments across all of them and currently reports clean.

---

# Second defect: `assert_observer_after_terminal` looks for an event that is never emitted

**Not fixed. Handed over.** Found by run 22 (`1a4c097d3`), the first run to reach
the post-terminal assertions.

## The code

`tests/support/workflow_decomposition_synthetic_evidence.rs:311-319`

```rust
let terminal_seq = events
    .iter()
    .filter(|event| event["kind"] == "completed")   // <- never matches
    .filter_map(|event| event["seq"].as_u64())
    .max()
    .expect("terminal event");                      // <- panics here
```

## The evidence

No run emits an event whose `kind` is `completed` — including runs that finish
`completed`. The terminal marker is `detail.event == "terminal_status"`, carried
on a `kind` that varies with the outcome:

| Workspace | Terminal status | Terminal event `kind` | seq | `kind == "completed"` | Observer seqs |
|---|---|---|---|---|---|
| `synthetic-1788366638` | `completed` | `stage_completed` | 64 | none | 65, 66 |
| `synthetic-1788390084` | `needs_review` | `stage_stalled` | 74 | none | 75, 76 |
| `synthetic-1788411432` | `needs_review` | `stage_stalled` | 77 | none | 78, 79 |

(The fixed decomposition uses `stage_failed` for the same `detail.event` on a
failed run — three kinds for one marker.)

**The property the assertion exists to prove is true in all three runs.** The
terminal event precedes the observer events every time. The assertion cannot see
it because it filters on the wrong field.

Like the preflight that counted its own `cargo` as competing work, this has never
executed — every earlier run died before reaching it, so the bug survived.

## Why this matters for sign-off

This assertion is where the real acceptance check lives: terminal-before-observer
ordering, `ObserveOnly` authority, at least one floor evaluated, at least one
finding, and the observer records naming `AC-SYN-001`. None of those clauses have
ever run. Whatever is downstream of line 319 is unverified.

## Shape of the fix, for review

Select the terminal event by `detail.event == "terminal_status"` rather than by
`kind`, since the kind encodes the outcome and the marker is outcome-independent.
Worth deciding at the same time whether the run kinds should emit a single stable
terminal event — the three-kinds-one-marker split is the same instability logged
as TD-012.

---

# Resolution — 2026-09-03

Both defects fixed. Reviewed and built by Fable; the code-pinning above is
what made a direct fix possible.

## Defect 1: one owner for the finding rules

`crates/archon-workflow/src/v2/review_findings.rs` now owns all three rules --
which arrays hold findings, when two findings are the same, which task a
finding belongs to -- and the machinery that used to duplicate them is gone:

| Was | Now |
|---|---|
| `v3_author_checks_b.rs` walk, identities, key, containment | deleted; the check calls `review_findings` |
| `lifecycle_policy/adversarial.rs::finding_identities` (a third Rust copy the audit missed) | delegates to `review_findings` |
| `v3_primitives.js` `reviewFindings` walk, `attributedMapFindings`, `findingIdentities`, `reattributeFindings`, `mergeMapAndReduceFindings`, `stampTaskIds`, `taskIdsOfOutcome` | deleted; `reviewFindings(env)` reads the host's attachment and nothing else |
| the offline replay (`/private/tmp/archon-replay/replay.mjs`) | obsolete; there is no second implementation left to reconcile |

**Mechanism.** When any call carrying a `reviewContract` completes, the script
host (`workflow_live_v2_script_host_exec.rs`, immediately after
`normalize_result_for_call`) calls
`review_findings::attach_host_review_findings`, which writes the finding set
into the result's `data.review_findings` before the record is saved and before
the script sees the result view. A `map` gets its branch findings attributed
to the task each branch reviewed; a `reduce_final` gets the attributed findings
of every map it named, merged with its own new findings, restatements dropped
by identity. The dry-run stub carries an empty attachment for the same calls,
so the rehearsal answers in the shape the runtime does.

`validate_review_accounting_from_reducers` is now host-against-host: the
accounting the script reports must equal, as a multiset by finding key, what
the host attached to the final reducer. It re-derives nothing. It still refuses
a script that filters or invents findings between reading and reporting them,
and it refuses a final reducer that named a map with no recorded result.

**Drift guard.** `the_prelude_carries_no_copy_of_the_finding_rules` fails the
build if the prelude ever again contains the findings-array key list, the
identity key list, or any of the deleted function names. The six JavaScript
attribution scenarios are ported verbatim to `review_findings_tests.rs`, plus
the walk, identity, merge and attachment behaviours, and the two refusals the
old containment could not express.

**A seventh divergence, found while porting.** The prelude keyed its
attribution table by the `item_id` it put on each map item, but the host forms
branch ids from `id`, `task_id` or `work_unit_id` -- never `item_id` -- so live
branch ids were `coverage-audit-map-0`, not `review-task-…`, and the table
matched no branch. Live attribution depended entirely on reviewers volunteering
task ids the map prompt never asked for. The host now reads the reviewed task
from the branch input it built itself.

Scripts authored before this change still run: `reviewFindings`,
`adversarialReview`, `coverageAudit` and `remediateFindings` keep their
signatures. The prelude source changed, so in-flight runs cannot resume across
the change (script digest); fresh runs only.

## Defect 2: the observer assertion

`terminal_event_seq` selects the terminal event by
`detail.event == "terminal_status"`, the marker every finalizer path stamps.
Two tests replay the events run 22 recorded, including a finished branch's
`stage_completed` ahead of the terminal `stage_stalled`, and the whole
post-terminal assertion executes offline against the recorded finalization and
observer records (`tests/fixtures/decomposition-synthetic/recorded-run-22`).
Sabotage: restoring the `kind` filter fails both. Fixed in `84aed38d3`.

One precision on the evidence above: `kind: completed` IS emitted, by
`emit_run_status_event` on `RunStatus::Completed` -- but only on the
`finalize_run_status` path, which no completing run takes. The marker is the
stable thing; the kind legitimately encodes the outcome for the TUI and web
API, and was left alone.
