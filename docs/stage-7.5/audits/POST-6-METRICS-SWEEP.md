# POST-6-METRICS-SWEEP — telemetry drift audit

> Ticket: `docs/stage-7.5/tickets/TASK-AGS-POST-6-METRICS-SWEEP.md`
> Pattern: AUDIT (read-only spot-check)
> Stage: 7.5 POST-STAGE-6 follow-up (sibling of #88-#93)
> Execution date: 2026-04-21
> Executor: worktree coder subagent (Gate 1 baseline + Gate 2 audit doc)

---

## Gate 1 — baseline-captured

Commands (exactly as specified in the ticket):

```
grep -rn "tracing::\|log::\|metrics::" src/command/ --include="*.rs" | wc -l
grep -rln "tracing::" src/command/ --include="*.rs" | wc -l
```

### Raw baseline numbers

| Metric | Spec expected | Actual (2026-04-21) | Delta |
| --- | --- | --- | --- |
| `-rn` total matching lines | 35 | **40** | +5 |
| `-rln` files with `tracing::` matches | 8 | **9** (+ 1 binary-skipped file) | +1 |
| Total handler `.rs` files in `src/command/` | 40 | **60** | +20 |

### Why the numbers differ from spec

Three distinct effects inflate/skew the raw greps relative to spec's 35/8 baseline. Each is mechanical, not a drift in production code:

1. **`src/command/tui_helpers.rs` is reported as a binary file by `grep`** (contains a `\0` byte around offset 2716, so default-text `grep` skips line-level matching). It is actually a regular Rust source file with 6 real `tracing::{info,warn}!` emissions (lines 23, 83, 97, 105, 116, 135). Verified via `grep -a` (force-text). Spec omits this file entirely.
2. **`src/command/add_dir.rs` contains only rustdoc comment references** to `tracing::info!` — zero actual emissions. The 5 grep hits at lines 97, 107, 114, 156 are module-level `//!` comments describing the effect-slot ordering that lives in `src/command/context.rs:393`. Spec includes this file in the 8-handler list based on the raw grep hit count; real emissions = 0.
3. **`src/command/registry.rs` comment references** at lines 403, 975, 977, 1052, 1598, 2834 also contribute to the 40-line total but are rustdoc/inline comments referring to tracing calls elsewhere. Only lines 984 and 990 are real emissions (inside `CommandContext::emit`).
4. **60 handler modules present, not 40** — the ticket spec's claim of "40 handler modules" is outdated (see `ls src/command/*.rs`). The delta has no bearing on migrated-telemetry drift; it only changes the "zero-telemetry" bucket size (32 → 52). Spec's Gate 3 row-count check for Section 3 ("exactly 32 handlers") is therefore not literally satisfiable — see Section 3 for the reconciled list.

### Real emission inventory (ground truth for this audit)

After classifying each raw grep hit as either an **emission** (runnable telemetry call) or a **comment reference** (rustdoc `//!`, inline `//`, or doc-string `///`), the real emitting footprint is:

- **8 files** emit telemetry (same count spec claims, but **different list** — see next paragraph).
- **26 emission sites** across those 8 files (not 35).

Spec list (8): `add_dir.rs`, `agent.rs`, `context.rs`, `ide_stdio.rs`, `pipeline.rs`, `registry.rs`, `remote.rs`, `utils.rs`.
Reality (8): `agent.rs`, `context.rs`, `ide_stdio.rs`, `pipeline.rs`, `registry.rs`, `remote.rs`, `tui_helpers.rs`, `utils.rs`.
Delta: `add_dir.rs` (spec → reality: dropped — only rustdoc references) and `tui_helpers.rs` (reality → spec: added — missed due to binary-file grep skip).

This is **audit-surface drift** (spec stale) not **production drift** (no migration lost a metric). No production code changes required.

### Test baseline

Per ticket instructions: skipped `cargo test`. Test count last verified ≥ 361 post-CANCEL-AUDIT; audit is pure-docs so ±0 tests delta is expected.

---

## Classification taxonomy

From the ticket's Acceptance Criteria (§2):

| Code | Meaning |
| --- | --- |
| `CLEAN` | Telemetry preserved identically through migration (or newly added by a POST-6 ticket by design). |
| `LOST_METRIC` | Pre-migration emission not preserved. |
| `STALE_BREADCRUMB` | Comment references a metric that was never emitted. |
| `LABEL_DRIFT` | Emission exists but label/key changed. |
| `THIN_WRAPPER` | Intentionally a no-op wrapper, no telemetry expected. |
| `DEFERRED` | Upstream intercept awaiting POST-STAGE-6 migration (e.g. /compact, /clear). |

---

## Section 1 — 8-handler telemetry table (real emissions)

One row per emission site. Handler column uses the `/slash-name` where the tracing call belongs to a specific command handler, or the module role where the file is infrastructural (registry builder, CLI helper, transport).

| # | Handler / role | file:line | level | target | fields / message summary | Classification |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | `DiscoveryCatalog::build_catalog` (`/agent list`) | `src/command/agent.rs:16` | `debug` | (default) | `loaded`, `invalid` kv + message `"agent scan"` | CLEAN |
| 2 | `handle_agent_search` (`/agent search`) | `src/command/agent.rs:59` | `debug` | (default) | `loaded`, `invalid` kv + message `"local scan"` | CLEAN |
| 3 | `apply_effect → AddExtraDir` (`/add-dir`) | `src/command/context.rs:393` | `info` | (default) | `dir = %path.display()` + `"added working directory via /add-dir"` (byte-identical to shipped `slash.rs:683`) | CLEAN |
| 4 | `apply_effect → SetEffortLevelShared` (`/effort`) | `src/command/context.rs:409` | `info` | (default) | `level = %level` + `"set effort level via /effort"` (additive — shipped /effort had no tracing; new by design per B11 rustdoc) | CLEAN |
| 5 | `apply_effect → SetPermissionMode` (`/permissions`) | `src/command/context.rs:433` | `info` | (default) | `mode = %resolved` + `"set permission mode via /permissions"` (additive — shipped /permissions had no tracing; new by design per B12 rustdoc) | CLEAN |
| 6 | `handle_ide_stdio_command` (`--ide-stdio` entry) | `src/command/ide_stdio.rs:10` | `info` | (default) | `"IDE stdio mode: session={session_id}"` | CLEAN |
| 7 | `handle_ide_stdio_command` error branch | `src/command/ide_stdio.rs:12` | `error` | (default) | `"IDE stdio error: {e}"` | CLEAN |
| 8 | `init_leann` (pipeline LEANN init failure) | `src/command/pipeline.rs:319` | `warn` | (default) | `error = %e` + `"LEANN init failed; continuing without code context"` | CLEAN |
| 9 | `init_leann` (pipeline LEANN ctor failure) | `src/command/pipeline.rs:324` | `warn` | (default) | `error = %e` + `"LEANN unavailable; continuing without code context"` | CLEAN |
| 10 | `CommandContext::emit` (full-channel drop) | `src/command/registry.rs:984` | `warn` | `archon_cli::command::tui` | `"tui_tx full (256-slot buffer saturated) — event dropped"` — added by POST-6-TRY-SEND | CLEAN |
| 11 | `CommandContext::emit` (closed-channel drop) | `src/command/registry.rs:990` | `error` | `archon_cli::command::tui` | `"tui_tx closed — TUI receiver task is dead"` — added by POST-6-TRY-SEND | CLEAN |
| 12 | `handle_remote_command` (SSH connect) | `src/command/remote.rs:31` | `info` | (default) | `"remote ssh: user={user} host={host} port={port} session_id={remote_session_id}"` (multi-line macro; block starts at :31) | CLEAN |
| 13 | `handle_remote_command` (SSH agent fwd flag) | `src/command/remote.rs:37` | `info` | (default) | `"remote ssh: agent_forwarding={} (from config.remote.ssh.agent_forwarding)"` (multi-line macro; block starts at :37) | CLEAN |
| 14 | `handle_remote_command` (WS connect) | `src/command/remote.rs:103` | `info` | (default) | `"remote ws: connecting to {url} session_id={remote_session_id}"` | CLEAN |
| 15 | `handle_list_output_styles` (deprecated path warn) | `src/command/tui_helpers.rs:23` | `warn` | (default) | `"Loading from deprecated path {}. Rename to {} to suppress this warning."` with `old_dir.display()`, `new_dir.display()` | CLEAN |
| 16 | `setup_voice_pipeline` (voice disabled) | `src/command/tui_helpers.rs:83` | `info` | (default) | `"voice: disabled (config.voice.enabled=false)"` | CLEAN |
| 17 | `setup_voice_pipeline` (toggle-mode wired) | `src/command/tui_helpers.rs:97` | `info` | (default) | `"voice: toggle_mode={} (hotkey action={:?})"` with `config.voice.toggle_mode`, `hotkey_action_for_mode(...)` | CLEAN |
| 18 | `setup_voice_pipeline` (audio device detect) | `src/command/tui_helpers.rs:105` | `info` | (default) | `"voice: real audio device detected (sample_rate={}, channels={})"` with `audio_capture.sample_rate`, `audio_capture.channels` | CLEAN |
| 19 | `setup_voice_pipeline` (audio fallback) | `src/command/tui_helpers.rs:116` | `warn` | (default) | `"voice: no audio device available, using mock audio source"` | CLEAN |
| 20 | `setup_voice_pipeline` (pipeline wired) | `src/command/tui_helpers.rs:135` | `info` | (default) | `"voice: pipeline wired (provider={}, device={}, hotkey={})"` with `config.voice.stt_provider`, `config.voice.device`, `config.voice.hotkey` | CLEAN |
| 21 | `apply_tool_filters` (whitelist applied) | `src/command/utils.rs:82` | `info` | (default) | `"tool whitelist applied: {} tools retained"` with `names.len()` | CLEAN |
| 22 | `apply_tool_filters` (blacklist applied) | `src/command/utils.rs:87` | `info` | (default) | `"tool blacklist applied: removed {} patterns"` with `names.len()` | CLEAN |
| 23 | `fetch_account_uuid` (success path) | `src/command/utils.rs:140` | `info` | (default) | `"fetched account_uuid: {}"` with 8-char prefix of `uuid` | CLEAN |
| 24 | `fetch_account_uuid` (missing uuid) | `src/command/utils.rs:144` | `warn` | (default) | `"profile response missing account_uuid"` | CLEAN |
| 25 | `fetch_account_uuid` (HTTP non-success) | `src/command/utils.rs:148` | `warn` | (default) | `"profile fetch failed: HTTP {}"` with `resp.status()` | CLEAN |
| 26 | `fetch_account_uuid` (transport error) | `src/command/utils.rs:152` | `warn` | (default) | `"profile fetch error: {e}"` | CLEAN |

**Section 1 row count: 26.** (This exceeds the ticket's specification of "8 rows" because the ticket's row unit was ambiguous — the AC says "Section 1 row count = 8 (one per handler with telemetry)" in Gate 3, but the Scope says "catalog every `tracing::` / `log::` / `metrics::` call". I have chosen the finer-grained per-call format because (a) the Scope clause is explicit, (b) per-file aggregation would hide label/message drift at the call-site level, which is the whole point of a drift sweep. An 8-row per-file summary follows for the Gate 3 reviewer's convenience.)

### Section 1 appendix — 8-row per-file summary (for Gate 3 cross-check)

| # | File | Emission count | All classifications |
| --- | --- | --- | --- |
| 1 | `src/command/agent.rs` | 2 | CLEAN ×2 |
| 2 | `src/command/context.rs` | 3 | CLEAN ×3 |
| 3 | `src/command/ide_stdio.rs` | 2 | CLEAN ×2 |
| 4 | `src/command/pipeline.rs` | 2 | CLEAN ×2 |
| 5 | `src/command/registry.rs` | 2 | CLEAN ×2 (both added by POST-6-TRY-SEND) |
| 6 | `src/command/remote.rs` | 3 | CLEAN ×3 |
| 7 | `src/command/tui_helpers.rs` | 6 | CLEAN ×6 |
| 8 | `src/command/utils.rs` | 6 | CLEAN ×6 |
| **Total** | **8 files** | **26 emissions** | **CLEAN ×26 — no drift** |

Notes on the 8-file list vs spec-8 list:

- Spec listed `add_dir.rs` as one of the 8. **Audit correction**: `add_dir.rs` has zero actual emissions (only rustdoc comment references to its migration-target `tracing::info!` call, which lives in `context.rs:393`). `add_dir.rs` is correctly classified as a zero-telemetry file; it is included in Section 3.
- Spec omitted `tui_helpers.rs`. **Audit correction**: `tui_helpers.rs` has 6 emissions; it is included here. It was missed in the spec baseline because default `grep` skips it as binary (embedded `\0` byte).

Neither correction implies a migration lost a metric. Both are classifications CLEAN.

---

## Section 2 — upstream intercepts (session.rs)

| # | Intercept | file:line | telemetry currently emitted | migration-target handler | Classification |
| --- | --- | --- | --- | --- | --- |
| 1 | `/compact` (bare + `/compact <arg>`) | `src/session.rs:2249` | **None.** The /compact branch at 2249-2261 emits only `TuiEvent::TextDelta` + `TuiEvent::SlashCommandComplete` via `input_tui_tx.send(..).await`; it contains zero `tracing::` / `log::` / `metrics::` calls in its body. | Pending POST-STAGE-6 migration (B24 did THIN-WRAPPER body-migrate per task #83 but the session.rs intercept remains). No target handler has absorbed the actual `agent.lock().await.compact(..).await` call yet. | DEFERRED |
| 2 | `/clear` | `src/session.rs:2265` | **Two `tracing::warn!` calls** in the personality-snapshot pre-clear block: <br>  — `src/session.rs:2292` → `tracing::warn!("personality: failed to save snapshot: {e}")` <br>  — `src/session.rs:2298` → `tracing::warn!("personality: failed to prune snapshots: {e}")` <br>  Both fire inside `if persist_personality { ... save_snapshot()/prune_snapshots() }` on error paths only. | Pending POST-STAGE-6 migration. When /clear is moved out of session.rs, the migrating ticket MUST preserve these two `tracing::warn!` calls verbatim (same message prefix `"personality: failed to save/prune snapshot"`, same `{e}` interpolation) to keep the operator-visible error log stable. | DEFERRED |

**Section 2 row count: 2.**

### Section 2 notes

- The /exit branch at `src/session.rs:2195` has the same personality-snapshot telemetry (lines 2222, 2228) — identical pattern to /clear. Not requested by the ticket spec (ticket scope covers /compact at 2249 and /clear at 2265 only), but noted here for the eventual /exit migration ticket.
- `tui_tx.send(TextDelta)` calls inside /compact and /clear are **not** `tracing::`/`log::`/`metrics::` — they are TUI event emissions. Excluded from this audit per ticket scope.

---

## Section 3 — zero-telemetry handler modules

### Why 52, not 32

Ticket spec (line 31) states "32 handlers have zero telemetry". Actual count of files in `src/command/*.rs`:

```
$ ls src/command/*.rs | wc -l
60
```

60 total files − 8 files with real emissions (see Section 1 appendix) = **52 zero-telemetry files**. The spec's "32" figure was based on a "40 handler module" base count that no longer matches the tree (worktree has 60 `.rs` files under `src/command/`; some of those are infrastructural modules like `dispatcher.rs`, `parser.rs`, `mod.rs`, `errors.rs`, `registry.rs`-adjacent helpers, `test_support.rs`, and so on — not all are user-facing slash-command handlers). The audit is a raw-file sweep, not a user-facing-command sweep, so 52 is the correct figure.

### The 52 zero-telemetry handler modules

All of the following files have no `tracing::`, `log::`, or `metrics::` emission sites (verified via `grep -an` including binary-text force and excluding comment-only matches where applicable). Every entry below is classification **CLEAN** (no pre- or post-migration telemetry).

```
add_dir.rs, background.rs, bug.rs, cancel.rs, checkpoint.rs, clear.rs,
color.rs, compact.rs, config.rs, context_cmd.rs, copy.rs, cost.rs,
denials.rs, diff.rs, dispatcher.rs, doctor.rs, effort.rs, errors.rs,
export.rs, fast.rs, fork.rs, garden.rs, help.rs, hooks.rs, login.rs,
logout.rs, mcp.rs, memory.rs, mod.rs, model.rs, parser.rs, permissions.rs,
plugin.rs, recall.rs, release_notes.rs, reload.rs, rename.rs, resume.rs,
rules.rs, sessions.rs, slash.rs, status.rs, task.rs, team.rs,
test_support.rs, theme.rs, thinking.rs, update.rs, usage.rs, vim.rs,
voice.rs, web.rs
```

(52 files, sorted alphabetically — classification CLEAN for all.)

#### File-list notes

- **`add_dir.rs`** — contains rustdoc `//!` comment references to `tracing::info!` but emits nothing itself. The actual emission lives at `src/command/context.rs:393` (captured as Section 1 row 3). No drift.
- **`denials.rs:283`** matched the raw grep for `log::` but this is `use archon_permissions::denial_log::DenialLog;` — a module path, not the `log` crate. Correctly classified as zero-telemetry.
- **`cancel.rs`** — explicitly a THIN-WRAPPER per the task #91 close notes; zero telemetry is the intended post-migration state. Included in the CLEAN bucket per the ticket taxonomy convention (ticket's `THIN_WRAPPER` code is reserved for the intercept-side row; handler-side THIN_WRAPPER bodies with no telemetry roll up as CLEAN in Section 3).
- **`compact.rs`**, **`clear.rs`** — the body-migrate handler files (task #83, B24). They are THIN-WRAPPER bodies; the actual logic still runs via the session.rs intercept (Section 2 DEFERRED rows). Zero telemetry in the handler bodies is by design.
- **`slash.rs`, `dispatcher.rs`, `parser.rs`, `registry.rs`-siblings** — infrastructural modules that don't emit per-command telemetry. `registry.rs` itself has 2 emissions (Section 1) for the shared `CommandContext::emit` fallback path, which is not a per-command telemetry site.

**Section 3 file count: 52.**

---

## Summary

- **Section 1 rows:** 26 emission-level rows (8 files × per-call detail). Appendix lists 8-row per-file summary for Gate 3 cross-check.
- **Section 2 rows:** 2 (/compact, /clear).
- **Section 3 file count:** 52 (zero-telemetry handlers).
- **All classifications:** CLEAN (Section 1) or DEFERRED (Section 2) or CLEAN (Section 3). **Zero `LOST_METRIC`, zero `LABEL_DRIFT`, zero `STALE_BREADCRUMB` findings.**

### Audit-surface corrections (non-production)

1. Ticket spec's "8 handler file" list includes `add_dir.rs` (incorrect — zero real emissions) and omits `tui_helpers.rs` (6 real emissions). Audit classifies both correctly; no prod code change.
2. Ticket spec's "35 total calls" undercount arises from `grep` skipping `tui_helpers.rs` as binary AND from miscounting rustdoc comment references as emissions. Real emission count: 26.
3. Ticket spec's "40 handler modules / 32 zero-telemetry" base count is stale — `ls src/command/*.rs` reports 60 files. Zero-telemetry count becomes 52.

None of these corrections require production code changes. They are spec-text updates that the metrics-sweep ticket itself can absorb without opening a follow-up ticket.

### Verdict

**PURE_AUDIT — no production code changes.** All migrated telemetry is CLEAN. Both DEFERRED intercepts (session.rs /compact :2249, /clear :2265) are known-pending per the POST-STAGE-6 roadmap; this audit documents the telemetry contract (two `tracing::warn!` personality-snapshot calls on /clear at lines 2292, 2298) that a future migration ticket must preserve.
