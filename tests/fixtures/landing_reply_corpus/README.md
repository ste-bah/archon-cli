# Landing reply corpus (#630, P4 D1/D2)

These are read-only source excerpts, not generated replies. Reply `.txt` files
contain the exact decoded assistant text, without an added newline. JSON tool
blocks are byte-for-byte excerpts of transcript JSON (outer collection brackets
and the final newline are corpus framing). `sources.json` lists individual tool
call ids and transcript turn indices. No source files were modified.

Sources live under `/Volumes/Externalwork/archon-cli/project-1/.archon/workflows/`:

| Run | Directory |
| --- | --- |
| R8 | wf-a4cfb5c4-6a90-42e9-a447-6f05558a763a |
| R10 | wf-5fd23e2f-f041-4a59-9fdf-32b9973f80ef |
| R14 | wf-58eb4c15-a040-4d18-8a71-3c0fe65f95be |
| R17 | wf-b349eb91-2475-4cbf-9173-b1e30dd0f505 |
| R18 | wf-e1d6ee46-c91b-4bfa-aefb-47d008079b4c |

Each `agent-outputs/skeleton-author-N.json` gives `transcript_directory` under
`~/.archon/sessions/<run>/subagents/`. The transcript has the original tool
inputs; the workflow output summaries omit them. Turn numbers below are zero
based and match D-doubles.md §1. Source final replies may differ from the host's
completed reply (notably R17 author-3).

| Shape / file stem | Source call and turn | Expected runner outcome; final requests |
| --- | --- | --- |
| exact | R18 skeleton-author-1 [248] | 19 new: final; 1 |
| exact_r14 | R14 skeleton-author-1 [250] | 16 new: final; 1 |
| under_count | R17 skeleton-author-3 [250] | receipt 1 vs 3 new: retries, completes 3; 3 |
| whole_store | R18 skeleton-author-2 [47] | receipt 19 vs 16 new: retries, completes 16; 3 |
| seed_only | R18 skeleton-author-3 [238] | 19 seeds, no new work: retries, refuses; 3 |
| prose_receipt | R18 skeleton-author-4 [176] | 13 new: completes 13; 1 |
| skeleton_r8 | R8 skeleton-author-1 [23] | no landings: retries, refuses; 3; with 2 new: completes 2; 1 |
| skeleton_r10 | R10 skeleton-author-5 [165] | no landings: retries, refuses; 3 |
| single_record | R10 skeleton-author-7 [220] | invalid `min_instances:1` field: retries, refuses; 3 |
| valid_record | R18 skeleton-author-1 tool input (sources.json) | scheduled as final text: lands, retries, receipt finishes; 2 |
| valid_record repeated | same input excerpt | lands once, bounded retries, completes 1; 3 (P1/A2) |
| extra_keys | R8 skeleton-author-2 [26] | retries, completes 14; 3 |
| list_receipt | R10 skeleton-author-6 [252] | list count: retries, completes 2; 3 |
| mixed.json | R18 skeleton-author-2 [15], call_cb4a93877a64113807d589c3 | tool lands before final receipt; 2 total |
| malformed | R10 skeleton-author-9 [247] | with 2 new: completes 2; 1 |
| text_tool | R10 skeleton-author-3 [127] | text executes no tool; then 3 empty finals refuse; 4 total |
| string_commands_run | R18 skeleton-author-2 [7], call_f8250115318c1e8e2c15e107 | refuses argument, retains other valid work; 4 total |
| string_evidence | R17 skeleton-author-2 [262], call_8612820668d9175290b1f6b5 | refuses argument, retains other valid work; 4 total |
| empty end_turn | synthetic boundary, D-doubles.md §3: not observed live | retries, refuses; 3 (A4 refuted on this base) |
| empty max_tokens | synthetic boundary, DEFECTS.md A7 | output-limit failure; 1 (P1/A7) |

`landings.json` contains the last accepted tool block for each of 19 distinct
subjects in R18 skeleton-author-1. Replay sends these through the real tool
registry, `SubagentRunner` landing judge, real `RecordLanding` via `Bridge`, and
`compose_raw_reply`. Seeds are task bytes copied from a disjoint prefix of these
records. This controlled setup isolates reply handling; it does not reconstruct
a whole historical workflow. Under-count uses 3 new + 16 seeds to preserve the
live reply's relationship to new and retained counts; the original R17 retained
18. No reply or tool input is rewritten to fit the setup.

Final request counts exclude the initial tool-only landing round unless marked
“total”. Successful compositions compare every task with the original task parsed as `FrozenTask`
(including all typed fields and their defaults); refusals inspect retained records and pending-rejection feedback.
Whole skeleton text must never replace retained work. String argument cases send
one additional valid landing before the invalid tool call and verify it survives.
The empty boundaries have no fabricated live fixture or source attribution.
`valid_record.txt` copies the input object bytes of the first accepted tool block;
its scheduling as final text is synthetic. The actual R10 final is also tested,
but cannot demonstrate successful landing because its contract key is invalid.

Known failures intentionally remain active on base 08daf24cb: P1/A2
`repeated_record_replies_complete_within_budget` and P1/A7
`max_tokens_empty_is_output_limit`. A4 is refuted here: `empty_end_turn_refuses`
already passes with three requests and an incomplete-artifact refusal. No product changes for P1/P2/P3
are included. The module is wired only in `src/command/workflow_stage_landing.rs`.

Validation on base 08daf24cb with this P4 change:
`--bin archon corpus_tests`: 20 passed, 2 failed, 0 ignored (P1/A2 and P1/A7
above). A2 made four requests instead of three; A7 returned an incomplete-artifact
error instead of an output-limit failure. `-p archon-core --lib landed`: 10 passed.
The D2 fake-host regression failed on seed-only completion before its fix, then
passed in the targeted suite. No pre-existing test assertion was weakened.
