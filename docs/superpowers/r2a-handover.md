# R2a proof — handover

**State:** the engine works end to end and is evidenced. The proof *test* has
never passed, and the reason is no longer a code defect: it depends on ~40
consecutive model calls all returning well-formed envelopes, and the observed
malformation rate makes a clean pass a coin flip at ~2.5 hours per attempt.

`HEAD = 0c3007f99`. 79 commits since the spec freeze `84b1d8466`. Nothing
pushed. `crates/archon-trading` untouched throughout
(`02dee5f0b2bbd41bc75abccf16dc4b47c1ef54daeea91ca0287bf552aa59871a`).
Unit suite: 2176 pass, 6 fail — all six pre-existing `command::trading_data`
from the uncommitted registry v1→v2 work.

## The evidence that the engine works

Run 18, preserved whole at
`/private/tmp/archon-r2a-workspaces/synthetic-1788366638`:

| | |
|---|---|
| Terminal status | `completed` |
| Write branches | 12 / 12 accepted |
| Calls | 21 / 21 accepted, zero needs_review |
| Blocking gaps | 0 |
| `src/alpha.txt` | created, 12 bytes |
| `src/beta.json` | created, 20 bytes |
| `.archon/proof/synthetic-observer-target.json` | absent — correct, the PRD forbids any task creating it |
| Frozen `AC-SYN-001` floor | byte-identical to `synthetic_floor()`, every serde default preserved |
| Decomposition findings | 13 log lines, all in the fixture-mandated chain |
| Repair loop | "AC-SYN-001 has no skeleton owner" raised on skeleton attempt 1 (`findings=2`), cleared on attempt 2 (`findings=1`) |

That is PRD → decomposition → two tasks implemented → verified → two adversarial
reviews → remediation → accounting → clean terminal, on the local qwen cluster
with no Anthropic API.

## Why the proof test still fails

Run 19 (`synthetic-1788376435`) ran the same code and produced 1 failed branch,
1 blocked, 4 needs_review calls. Every gap has the same root:

```
schema repair failed after bounded retries:
  root=agent output must be one JSON WorkflowV2Result object:
       key must be a string at line 2 column 7;
       output begins: ``` export const meta = { name: 'synthetic-alpha-beta-chain', ...
  last=... missing field `path`
```

The model returned a markdown-fenced **JavaScript** block instead of the JSON
envelope (twice, on `author-workflow-script`), and an invalid branch envelope on
`coverage-audit-map`. The repair path re-asks rather than repairing syntax, and
exhausts its bounded retries.

**This is the remaining work.** Not a workflow defect — an envelope-robustness
defect. Two shapes are recorded and reproducible:

1. **Fenced non-JSON.** Reply is ```` ```\nexport const meta = { ... ```` . The
   scan finds the `{` of the JS object literal and fails with "key must be a
   string". A JS object literal is not recoverable as JSON, so the correct
   handling is to detect it and re-ask with a sharper instruction, not to parse.
2. **`missing field 'path'`.** The repair produced an envelope whose
   `files_changed` entry lacks `path`. Worth checking whether the repair prompt
   shows the required shape for that field.

One shape of this family is already fixed: a trailing comma
(`agent_output_normalize.rs`, `strip_trailing_commas`, four tests).

**Test these offline.** The failing outputs are on disk under each workspace's
`.../agent-outputs/` and in the `v2/results/*.json` records. There is no reason
to spend a live run on them.

## Tools built for this

- `/private/tmp/archon-replay/replay.mjs` — replays the accounting
  reconciliation (host walk + prelude merge) against every preserved run in
  under a second. Found two defects that would each have cost a 2.5-hour cycle.
  Currently reports **clean on all recorded runs**.
- `ARCHON_R2A_KEEP_WORKSPACE=<dir>` — the proof harness keeps its workspace
  instead of deleting a `TempDir` on failure. Three diagnoses were lost before
  this existed.

## Defects fixed since the audit

Seven in shared workflow code, all found by running rather than reading, all in
code the audit had already passed over:

| # | Defect | Effect |
|---|---|---|
| 1 | Prelude read findings from one key; host unions three, recursively | accounting always a subset → run refused |
| 2 | Dry-run stub failed `usable()`, the predicate the contract mandates | scripts following the guidance rejected at pre-flight; hand-rolled ones passed |
| 3 | Host double-counted fan-out findings (`items` and `outcomes` are the same branches) | demanded a copy that never existed |
| 4 | Host compared findings byte-for-byte while the prelude enriches them | enriched findings reported missing |
| 5 | `Object.assign({}, "AC-SYN-001", …)` shredded a bare-string finding into `{"0":"A",…}` | coverage findings lost |
| 6 | Host demanded reduce findings the prelude drops by design ("restatements dropped by identity") | two identity notions, same class as 1 and 4 |
| 7 | A branch naming its file by the project path read as a scope escape | healthy run killed on `failure_kind: safety` |

Plus `remediateFindings` dispatching a write agent with no target files, which
threw and killed a completed 2h39m run at the last stage (TD-001, the real
mechanism — the earlier fix keyed on `attributable_to_task`, which nothing
emits).

## Proof harness defects fixed

The proof could never have passed, for reasons unrelated to the engine:
preflight counted its own `cargo` as competing work; the scratch project had no
provider endpoint and no `[models.anthropic]` aliases (so every author call
asked litellm for `claude-sonnet-4-6` and got HTTP 400); it required a global
binary install this project does not use; the workspace was deleted on failure;
and three assertions encoded invariants the fixture contradicts.

## Open in the ledger

`docs/superpowers/tracked-defects.md`, TD-001…TD-012. Unfixed and deliberate:

- **TD-012** — a healthy implementation run ends `Completed` or `NeedsReview`
  depending on reviewer whim. Legitimate under the `ObserveOnly` pin, but a
  status that does not distinguish two outcomes carries no information.
- **TD-006 / TD-010 partial** — `DecompositionPhaseStarted`,
  `ModelCallInFlight` and `ShadowFindingsObserved` now emit; status still lacks
  finalization/observer detail.

## Honest assessment

The audit I ran against the spec produced 11 findings and missed all seven
defects above. It compared code to a document; every one of these is two
components disagreeing about the same data, which only shows up when the code
runs. Reading was the wrong instrument and I presented its output as thorough.

I also introduced three defects while fixing others (removed an asserted event,
collided with the log's marker discriminator, broke the log grammar with
finding text), and mislabelled a `NeedsReview` terminal as `run_failed` in the
same change that fixed terminal failures having no reason at all.
