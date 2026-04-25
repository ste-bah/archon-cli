# FileSizeGuard

Enforces NFR-FOR-D4-MAINTAINABILITY: no `*.rs` file under the archon-cli repo
may exceed **500 lines**, except entries in `scripts/check-file-sizes.allowlist`.

Spec reference: `project-tasks/archon-fixes/agentshit/02-technical-spec.md`
§1424 ("FileSizeGuard") and §955 ("Every file ≤ 500 lines").

## When to run

- Locally before every commit that touches Rust source.
- In CI (wired by TASK-AGS-007 — not this task).
- Manually after any refactor that moves code between files.

```bash
bash scripts/check-file-sizes.sh
```

Exit code `0` means all non-allowlisted files are within the threshold.
Exit code `1` prints the offenders; fix them or (last resort) escalate.

## Allowlist state at phase-0 baseline

The allowlist currently grandfathers **68 files** across 12 crates
(main.rs + 67 others). This was larger than the original TASK-AGS-002
spec anticipated: the spec assumed main.rs was the sole >500-line file,
but phase-0 baseline execution discovered substantial pre-existing
NFR-FOR-D4 debt:

- `crates/archon-core/src/agent.rs`: 3593 lines
- `crates/archon-tui/src/app.rs`: 1767 lines
- `crates/archon-core/src/subagent.rs`: 1753 lines
- `crates/archon-core/src/agents/loader.rs`: 1646 lines
- ...60 more entries

Steven explicitly approved the expanded allowlist on 2026-04-11 to
preserve the guard's forward-looking purpose: **catch NEW >500-line
files while phases 4+ progressively refactor existing debt**.

## Shrinking the allowlist

Every phase-N task that refactors a file below 500 lines MUST, in the
same commit:

1. Confirm the file's new line count is ≤500.
2. Remove its entry from `scripts/check-file-sizes.allowlist`.
3. Run `bash scripts/check-file-sizes.sh` — it must exit 0.

The allowlist is not permanent storage. Its only job is to keep the guard
green during staged refactors. A PR is considered progress if — and only
if — the allowlist shrinks.

## What to do when a legitimate >500-line file is needed

**Do not add entries to the allowlist.** The default answer is: split the
file. Shared module boundaries, submodules, or extracting traits almost
always work.

If you genuinely believe a file must exceed 500 lines (e.g. generated code,
large table of constants, FFI bindings), escalate:

1. Open a discussion in the repo with the proposed file and reason.
2. Get explicit approval in review.
3. Only then add it to the allowlist with a comment citing the approval.

Silently growing the allowlist is a regression against NFR-FOR-D4 and will
be flagged in adversarial review.
