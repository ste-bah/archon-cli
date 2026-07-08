# FCDP — Fable Console Drafting Protocol (archon integration)

Gate-enforced drafting pipeline for dissertation prose, ported from the god-agent
FCDP v2 protocol and validated end-to-end in a standalone sandbox before promotion.

## Layout

| Piece | Where | What |
|---|---|---|
| `archon-draft` crate | `crates/archon-draft/` | Pack schema + G-P validator, «Qnn» quote substitution, stylometric measurement over `archon-lanham`, two-tier G-A comparator. Binary: `archon-fcdp` (measure / gp-validate / substitute / ga-gate) |
| Orchestration | `scripts/fcdp/` | Stage runners (D1 plan → D1.5 skeleton → D2 movement-by-movement), judge gates G-E/G-G (lean-context Fable calls, randomized binary batteries, free-prose extraction), mechanical gauntlet G-B/C/D/F, hash-chained provenance + AI-use declaration generator, R-loop drivers |
| Gate data | `crates/archon-draft/data/` | Locked G-A gate config (variance-derived, feature-tiered bands; MA-applications register), band-derivation candidates, exemplar pool |

## Pipeline

```
pack (G-P) → D1 plan (G-1) → D1.5 skeleton (G-1.5)
→ D2 draft with «Qnn» markers (movement by movement)
→ substitute (exit≠0 = G-B fail) → G-B/G-C/G-D/G-F (mechanical)
→ G-A (Tier-1 per-section; Tier-2 + T2-derived labels at chapter scale)
→ G-E foundation-fidelity + G-G consistency (fresh lean-context judge calls)
→ R-loop ≤3 (repair prompts restate the full movement plan)
→ hash-chained provenance → generated AI-use declaration
```

Validated 2026-07-07/08: a live dissertation section ran the full pipeline to
ALL-GATES-GREEN (4 PDF-verified quotes, 4 graded evidence items, 4 cycles incl.
instrument fixes; provenance chain verified; declaration generated).

Key operational notes:
- `claude-fable-5` via API: thinking is adaptive-only — set `output_config.effort`
  and generous `max_tokens`; always hard-abort on empty text output.
- Quotes must be PDF-verified in the pack-assembly session (`FCDP_TODAY` pins the
  session date for reproducing past runs).
- Judge calls receive only draft + rubric + the pack fields the rubric names
  (G-G additionally receives the D1 claim list, per FCDP §5).

Follow-ups (not in this change): `archon draft` subcommand surface; Rust port of
the Python orchestration; CozoDB promotion of the JSONL provenance chain via
`archon-provenance`.

Provenance of the design: `plans/fcdp-archon-port-plan-v2-2026-07-07.md` and the
FCDP v2 protocol document in the god-agent repo (claudeflow-testing).
