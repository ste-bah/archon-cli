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
