# Archon core crates audit — 2026-07-11

Scope: all core crates. Excluded: archon-workflow, archon-trading.
Each item: **problem → fix**. Ordered by impact.

## Executive summary

49 findings + 2 refactor plans (config.rs split; >500-line file splits). Five systemic patterns account for most of the P0 list — fixing the pattern once and migrating call sites beats fixing findings one-by-one:

1. **Rebuild/reopen per operation** — HNSW index rebuilt per query, RocksDB/Cozo handles opened per call, schema re-created per event, reqwest clients built per request (findings 1–5, 13, 22, 24, 35).
2. **Full scan instead of index** — Cozo filters on non-key columns scan whole relations: leann per-file, kb dedup per-chunk, memory keyword search per-query, knowledge retriever per-search (8, 9, 14, 16, 36).
3. **One transaction per row** — leann chunks, session messages, kb nodes, kb edges (14, 15, 16, 33).
4. **Blocking in async** — docs search pipeline, cozo retry sleeps, crossterm poll, cargo-build gate, ffmpeg (5, 12, 23, 38, 44, 46).
5. **Windows parity gaps** — hooks `sh -c`, `/bin/bash`, env passthrough, `/`-only path matching (21, 23, 47).

Biggest single wins: **#1** (persistent HNSW → search latency stops growing with corpus), **#17** (real Anthropic streaming → time-to-first-token), **#11** (remove global Cozo serialization → whole-app concurrency), **#40/41** (stop the rules-engine feedback loop polluting every prompt).

## Implementation notes (for Codex)

**Suggested PR batching** (each independently shippable, ordered by value):
- **PR 1 — Search stack:** findings 1–5, 28 (docs crate + docs_runtime). Verify: `archon docs search` twice on a >1k-chunk corpus; second query must not rebuild (add a `tracing::info` on rebuild path; assert latency drop).
- **PR 2 — Anthropic streaming:** 17–19. Verify: TUI shows tokens incrementally; kill network mid-stream → visible error, not silent truncation.
- **PR 3 — Cozo layer:** 11–13. Touches everything; run the full workspace test suite. Watch for tests that implicitly relied on the global write lock for cross-DB ordering.
- **PR 4 — Rules/corrections loop:** 40–43 (consciousness + archon-core caller).
- **PR 5 — Batch writes + scans:** 14–16, 33 (leann, session, kb). Shared helpers from cross-cutting rec #1 land here.
- **PR 6 — Platform/tools hardening:** 20–27, 47 (panics, Windows shells, unbounded buffers, timeouts).
- **PR 7 — TUI event loop:** 44–45. Do after PR 2 (streaming fix makes the frame-rate cap visible/testable).
- **Refactor PRs (config.rs, file splits, test-mod extraction):** pure motion only — no signature or behavior changes mixed in; land whenever convenient, ideally before PRs that touch the same files.
- **PR 8 — World-model inference inputs (W1, W3):** build the runtime `TraceWindow` from real session transitions instead of the prompt echo; reclassify task from planned tool calls. Verify: `latent_surprise` mean drops on the next eval run vs baseline.
- **PR 9 — Guardrail enforcement (W2, W4):** wire `required_actions` → archon-completion verification gates; guard Risky/Dangerous tool calls via the ToolRun surface. Do after PR 8 — enforcement should wait until predictions have credible inputs.
- **PR 10 — TUI output integrity (T1–T3):** delta coalescing instead of shedding, u32 scroll math, unicode-width wrapping. T1 lands with or before PR 2 — the streaming fix changes burst behaviour and both must be verified together (verify: paste a 5k-line tool output + stream a long response; transcript must be byte-complete and scrollable to both ends; `dropped_content == 0`).
- **PR 11 — Thinking & transcript UX (T4–T7):** always-capture thinking with rolling collapsed preview, transcript markers for thought blocks and tool calls, expanded-thinking height cap, scroll-lock hint. After PR 7 (event-loop rewrite) to avoid rebasing render changes twice.
- **Roadmap items R1–R8 and W5–W6** are design work, not defect fixes: gate them behind R0 (findings 9, 11, 17, 40–43) and land each with its R8 metric so the effect is measurable.

**General rules:**
- Line numbers reference commit `master @ 2026-07-11` (v1.3.11); re-grep the quoted identifiers if drifted.
- Every fix PR: `cargo clippy --workspace` clean at threshold 60 (clippy.toml) + `cargo test -p <touched crates>`. There is currently no Windows CI — findings 21/23/47 can only regress silently; consider adding a `windows-latest` job.
- Numbers like "~3 min worst case" are reasoned from code, not profiled. Where a fix claims a perf win (1, 3, 11, 14, 15), capture a before/after timing in the PR description.
- Several findings share code paths; if a later PR's context contradicts an earlier finding's assumption, trust the code and note the deviation rather than forcing the written fix.

---

## P0 — Search / vector stack

**1. HNSW rebuilt from scratch on every semantic search; persistent snapshot never loaded.**
`archon-docs/src/retrieval_semantic.rs:89` always calls `DocVectorStore::search_in_memory` (`vector_store.rs:158`), which full-scans and decodes every vector in RocksDB (`iter_records`) and rebuilds the whole `Hnsw` (ef_construction=200) per query. `build_hnsw` (`vector_store.rs:127`) already dumps a snapshot via `hnsw.file_dump` and writes `manifest.json`, but nothing in the repo ever loads it — no `HnswIo` usage exists.
**Fix:** In `rocksdb_hnsw_search` (`retrieval_semantic.rs:68`), when `latest_hnsw_manifest(provider)` returns Some and `manifest.vector_count == count_prefix(vec-prefix)`, load the dumped index with `hnsw_rs::hnswio::HnswIo::new(&hnsw_dir, &manifest.dump_basename)` and search it; fall back to `search_in_memory` only when there is no snapshot or it is stale. Requirements: (a) reverse id map — at `put_vectors` time also write `rid/{provider}/{hnsw_id}` → chunk_id keys so hits resolve without scanning; (b) cache the loaded index in a `OnceLock`/`Mutex<HashMap<(provider, dump_basename), Arc<Hnsw>>>` so repeat queries in one process don't re-read the dump.

**2. Snapshot is never auto-built.** `build_hnsw` is only reachable via manual CLI `src/command/docs_vector.rs:70`.
**Fix:** after `index_pending_chunks`/`reindex_all` complete with newly stored vectors (`archon-docs/src/indexing.rs`), call `build_hnsw(provider, dim, None)` so the snapshot tracks ingest. Delete older dump files for that provider after a successful manifest write (they currently accumulate forever).

**3. Every query does a second full scan just to count.** `vector_store.rs:116` `count_vectors` = `iter_records().len()` (loads + decodes every vector); called per query at `retrieval_semantic.rs:81`.
**Fix:** implement it as `self.count_prefix(&vector_prefix(provider))` (already exists, `vector_store.rs:225`).

**4. RocksDB store and Cozo docs DB opened per query / per tool call.** `retrieval_semantic.rs:74` `DocVectorStore::open_default()` per search; `archon-tools/src/docs_runtime.rs:17` `open_docs_db(ctx)` per tool call.
**Fix:** cache both behind `OnceLock` keyed by path (needed anyway for the index cache in #1).

**5. Doc tools run the whole blocking search pipeline on the async runtime thread.** `docs_runtime.rs:12 run_search` is `async` but synchronously does: open DB → fastembed ONNX query embedding → RocksDB scans → HNSW build. No `spawn_blocking` anywhere in archon-tools.
**Fix:** wrap the body of `run_search`/`run_answer`/`run_ingest` in `tokio::task::spawn_blocking`.

**6. KB Q&A never uses embeddings — semantic search is unimplemented.** `archon-pipeline/src/kb/query.rs:206` doc-comment claims "When embedder is available, uses HNSW vector search", but `search_nodes` only does a substring scan; `QueryEngine::embedder`, the `QueryEmbedder` trait and `with_embedder` are never used anywhere (grep-verified).
**Fix:** add a vector column + HNSW index on `kb_nodes` (mirror `kb/schema.rs` patterns from `archon-memory/src/vector_search.rs`), embed nodes at ingest, and in `search_nodes` use `self.embedder` when present; construct the engine with an embedder at call sites. Otherwise delete the dead trait/field.

**7. KB text search prefilter is case-sensitive while scoring is case-insensitive.** `kb/query.rs:224` Cozo `str_includes(title, $q)` — query "rust" never matches a node titled "Rust"; rows are dropped before the lowercase scoring below.
**Fix:** filter on `str_includes(lowercase(title), lowercase($q))` (Cozo `lowercase`), or fetch candidates via FTS index.

**8. archon-knowledge retriever loads every chunk into memory on every search.** `hybrid_retriever.rs:191 filtered_chunks` → `store::list_doc_chunks(db)` (full contents) per query; `exact_results` then re-tokenizes the whole corpus. With `document_filter`, `semantic_results` (`:134`) calls `list_doc_chunks` a second time just for `.len()` and sets HNSW `k = total chunk count`, degrading ANN to a full retrieval.
**Fix:** semantic mode: take top-k ids from HNSW and resolve only those chunks by id. Exact mode: add a Cozo FTS index on chunk content and query it. document_filter: over-fetch `k*4` and post-filter instead of `k = N`.

**9. archon-memory keyword search full-scans all memories per query.** `hybrid_search.rs:89` and `search.rs:56,127` call `read_all_memories` and score in Rust (code already logs `warn_full_scan`).
**Fix:** create a Cozo FTS index (`::fts create memories:content_idx {...}`) over content/title/tags at schema init and query it; keep the scan only as fallback when FTS is unavailable.

**10. Inconsistent cosine-distance→similarity conversion skews hybrid ranking.** `archon-memory/src/hybrid_search.rs:167` uses `1.0 - distance` while archon-docs/leann/knowledge use `1.0 - distance/2.0` (Cozo cosine distance ∈ [0,2]). Vector scores are systematically compressed vs keyword scores in the alpha blend.
**Fix:** use `1.0 - distance/2.0` here too; add a shared helper.

## P0 — Storage layer (Cozo)

**11. All Cozo operations are serialized process-wide.** `archon-cozo/src/lib.rs:184` `catch_guarded_operation` locks a global `COZO_PANIC_HOOK_LOCK` around **every** guarded op (reads included, all databases) because it swaps the process panic hook per call; `lib.rs:140` `COZO_PROCESS_WRITE_LOCK` additionally serializes writes across unrelated DB files. Net effect: effectively single-threaded DB access for the whole app.
**Fix:** install one silent panic hook once at startup that consults a `thread_local!` "in-cozo-op" flag before suppressing; drop the per-op hook swap and its mutex. Replace the single write mutex with a per-db-path map: `LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>>` keyed by `write_lock_path`.

**12. Retry loop blocks threads with `thread::sleep` for up to ~3 minutes.** `lib.rs:106` — 90 attempts × up to 2s backoff, on the calling (often async) thread.
**Fix:** add `run_guarded_async` using `tokio::time::sleep` and use it from async call sites; cut default attempts to ~10 for interactive paths. Also tighten `is_retryable_cozo_error` (`lib.rs:124`): `"code 5"` substring also matches "code 500" etc., so permanent errors can retry for minutes — match sqlite busy/locked codes precisely.

**13. Runtime event recorders open the learning DB and re-run schema creation per event.** e.g. `src/runtime/sandbox_events.rs:6-17` (`open_sqlite_guarded` + `ensure_learning_schema` + insert, per event); same pattern in `permission_events.rs`, `provider_observer.rs`, `provider_fallback_events.rs`, `codex_app_server_limits.rs`.
**Fix:** one `LazyLock<Result<DbInstance>>` for the learning DB with `ensure_learning_schema` run once; recorders reuse the handle.

**14. LEANN indexing is O(files × chunks) with one transaction per chunk.**
- `archon-leann/src/indexer.rs:534`: one `run_script` `:put` per chunk → thousands of transactions per repo index. Batch: build a multi-row `DataValue::List` param and one `:put` per 64-chunk embed batch.
- `indexer.rs:403 file_hash_matches` and `:422 remove_file_chunks` filter on `file_path` which is not the key → full relation scan per file. Fix: maintain a `leann_file_hashes {file_path => file_hash}` relation (or `::index create code_chunks:by_file {file_path}`) and query that.
- `indexer.rs:295` deletes a file's old chunks during the walk, but new chunks are only written at the very end (`:322`) — cancel/crash in between silently drops those files from the index. Fix: delete+insert per file adjacent (same script ideally).

**15. Session persistence writes one transaction per message after every turn, non-atomically.** `archon-session/src/storage.rs:396 replace_messages` loops `run_script` per message; crash mid-loop = mixed old/new state.
**Fix:** single script: pass all rows as one `DataValue::List` param, `:put` them and delete the stale tail in the same transaction. Also `storage.rs:445 truncate_messages_after` unconditionally sets `message_count = keep_up_to + 1`, inflating the logical count when called with `keep_up_to` ≥ actual — clamp to the current count. `list_sessions` (`:532`) does 2 extra queries per session (name/parent) — join in one script.

**16. KB ingest: dedup check full-scans per chunk; one transaction per node.** `archon-pipeline/src/kb/ingest.rs:198 hash_exists` scans `kb_nodes` for every chunk (content_hash is not the key) → O(chunks × nodes).
**Fix:** load existing hashes once per `store_chunks` into a `HashSet`, batch inserts into one script.

## P0 — LLM layer

**17. Anthropic "streaming" buffers the entire response before emitting anything.** `archon-llm/src/anthropic.rs:265` `spawn_stream_reader` does `response.text().await` and only then parses SSE — time-to-first-token equals full completion time. Every other provider does it right (`providers/openai.rs:185`, `openai_compat.rs:439`, `vertex.rs:192`, `bedrock.rs:155`, `local.rs:173`, `codex/client.rs:373` all use `bytes_stream()`).
**Fix:** port the incremental `bytes_stream()` + line-buffer loop from `providers/openai_compat.rs` into `spawn_stream_reader`; `split_sse_lines`/`parse_sse_event` already exist.

**18. Mid-stream network failures are silently swallowed.** Same line: `.unwrap_or_default()` — a dropped connection yields an empty/truncated stream with no error event; the agent sees a truncated response as if complete.
**Fix:** on read error, send `StreamEvent::Error { error_type: "network", message }` before closing the channel (follows naturally from the #17 rewrite).

**19. Full request body logged at INFO on every request.** `anthropic.rs:139`.
**Fix:** downgrade to `debug!`/`trace!`; confirm `debug_body` truncates/redacts user content.

## P1 — Correctness / panics / platform

**20. Panic on multibyte text.**
- `archon-pipeline/src/kb/query.rs:407` `&question[..97]` — non-ASCII question at that boundary panics `file_answer`. Fix: `question.chars().take(97).collect()`.
- `archon-session/src/listing.rs:24` `&name_display[..15]` and `:38` `&working_directory[len-25..]` — `archon session list` panics on non-ASCII session names or paths. Fix: shared char-based truncate helper.

**21. Hooks are hardcoded to `sh -c` — broken on plain Windows.** `archon-core/src/hooks/executor.rs:335` `Command::new("sh")`. On Windows without sh in PATH every hook fails to spawn.
**Fix:** resolve the shell once (`which::which("sh").or bash`), fall back to `cmd /C` on Windows; or add a `shell` field to `HookConfig`.

**22. Blocking hooks fail OPEN.** `hooks/executor.rs:154,204` — if a PreToolUse guard hook fails to spawn or times out, the result is `HookResult::allow()`, so a broken guard silently permits the tool call.
**Fix:** add per-hook `on_failure: allow|block` (default `block` for hooks whose purpose is gating), and return `HookResult::block(...)` on spawn error/timeout in that mode. Also `executor.rs:111` builds a fresh `reqwest::Client` per HTTP hook — use a `LazyLock<Client>`.

**23. Bash tool is unusable on Windows and can OOM.** `archon-tools/src/bash.rs`:
- `:168` `Command::new("/bin/bash")` hardcoded; plus `env_clear()` with a Unix-only passthrough list strips `SystemRoot`/`USERPROFILE`/`PATHEXT` on Windows. Fix: resolve bash via PATH (Git Bash on Windows), add Windows env passthrough set.
- `:281 spawn_pipe_reader` `read_to_end` buffers unlimited output; the 100KB `max_output_bytes` cap is applied only after. Fix: bounded reader — stop storing past the cap but keep draining the pipe.
- `:274 effective_timeout_ms` = `requested.max(configured)` with default 86,400s: the model can never shorten a timeout and a hung command stalls a turn for 24h. Fix: default ~600s; honor a shorter explicit request, clamp to a configured maximum.
- `:266` `classify_command(&command, &[], &[], &[])` passes empty user allow/deny lists — operator-configured rules never reach classification here; thread the config lists into `BashTool`.

**24. WebFetch buffers unbounded bodies and rebuilds its client per call.** `archon-tools/src/webfetch.rs:134` `response.bytes().await` before the 1MB truncation; `:98` new client per call.
**Fix:** `LazyLock<Client>`; read via `bytes_stream()` and stop at `MAX_BODY_BYTES`.

**25. Command classifier: `find` is unconditionally Safe.** `archon-permissions/src/classifier.rs:19` — `find / -delete` / `find -exec rm {} \;` auto-approve as Safe.
**Fix:** classify `find` Safe only when the unquoted text contains no `-delete`/`-exec`; otherwise Risky/Dangerous.

**26. KB URL and PDF ingest silently succeed doing nothing.** `archon-pipeline/src/kb/ingest.rs:88 ingest_url` returns `Ok(default)`; `:72 ingest_pdf` reads binary PDF via `read_to_string().unwrap_or_default()` → empty → `Ok(default)`.
**Fix:** return explicit errors, or delegate PDF to `archon-docs/src/pdf.rs` extraction and URL fetch to the WebFetch client — both already exist in-repo.

**27. Unsound `unsafe impl Sync`.** `archon-memory/src/embedding/local.rs:84` `unsafe impl Sync for LocalEmbedding`. If `fastembed::TextEmbedding` were `Send`, `Mutex` would already make the struct `Sync` and the impl would be dead code; its being required implies `TextEmbedding: !Send`, and asserting `Sync` over a `Mutex<!Send>` is unsound.
**Fix:** remove the impl; if it then fails to compile, move the model onto a dedicated thread and talk to it via `std::sync::mpsc` channel.

## P2 — Efficiency / hygiene

**28.** `vector_store.rs:173` hit→chunk resolution is `records.iter().find()` per hit (O(k×N)); `hybrid_retriever.rs:177` and `merge_results:91` same linear-find pattern; `exact_score:209` uses `Vec::contains` per term. Use `HashMap`/`HashSet`.
**29.** `archon-docs/src/indexing_parallel.rs:55` embeds **all** batches into a Vec before any write — no pipelining, whole-corpus vectors resident; `:94 handle.join()` errors silently drop a batch. Use a channel (embed workers → single writer) and log join failures.
**30.** `archon-leann/src/indexer.rs:188 index_repository` is `pub async` but fully blocking (walk+embed+DB) — wrap in `spawn_blocking` or verify all call sites already do.
**31.** `archon-memory/src/hybrid_search.rs:159` swallows vector-search errors (`Err(_) => empty map`) — silent degradation to keyword-only; log it. `hybrid_search:68` fetches final memories one query per id — batch.
**32.** `archon-memory/src/access/client_impl.rs:17` `block_in_place + block_on` panics on a current-thread runtime (comment acknowledges) — guard with `Handle::try_current()` runtime-flavor check and fall back to a scoped runtime.
**33.** `archon-pipeline/src/kb/query.rs:461` one script per DerivedFrom edge — batch.
**34.** archon-learning `store.rs` list functions filter on non-key columns (full scans). Fine at current volumes; add `::index create` on `created_at`/`event_type` before the ledger grows.

## P2 — Addendum (second-pass coverage of remaining crates)

**35. Activity JSONL sink re-opens the file per event.** `archon-observability/src/activity.rs:323 append_activity_event` does `create_dir_all` + `OpenOptions::open` for every event — at least 2 events per tool call, synchronous file IO inside the dispatch path.
**Fix:** hold `Mutex<BufWriter<File>>` opened at `JsonlActivitySink::new` (create dir there once); write + flush per event.

**36. Research pipeline multiplies the memory full-scan.** `archon-pipeline/src/memory.rs:139 recall_for_agent` / `:155 search_for_prompt` call `MemoryGraph::recall_memories` (full-table keyword scan, finding #9) once per agent — the 47-agent research pipeline does 47+ full scans per run. The FTS fix in #9 resolves this; no separate change needed, but re-test pipeline latency after.

**37. SSE line buffer unbounded.** `archon-mcp/src/sse_reconnect.rs:250` — a server that streams bytes without newlines grows `buf` without limit.
**Fix:** cap the buffer (e.g. 1 MB); on overflow, log and return `PumpOutcome::StreamEnded`.

**38. No timeout on ffmpeg/whisper subprocesses.** `archon-video/src/asr.rs:57` `extract_audio_track` and `:156` `WhisperCppAdapter::transcribe` await `.output()` with no timeout — a hung binary stalls video ingestion forever.
**Fix:** wrap in `tokio::time::timeout` (e.g. 10 min) and kill on expiry.

## Finding 50 — subagent context-overflow / failed-compaction death spiral (doc-heavy work)

Observed symptom: subagents ingesting documents (e.g. repo review) run out of context, then compaction fails. Verified at `audit/core-remediation-2026-07` tip `228f95dc`. Four stacked causes:

**50a. The compaction trigger lags one round behind burst growth.** `request_round.rs:298 current_trigger_tokens` *prefers* `last_known_context_tokens` (real usage from the **previous** round) over the fresh estimate whenever it's non-zero. A doc-review round appends several huge tool results (file_read has no byte cap — issue #75 A1 note), but the pre-round threshold check still sees last round's small number → no proactive compaction → the oversized request goes to the provider and hard-fails.
**Fix:** `max(last_known_context_tokens, trigger_tokens(messages))` — one line; the fresh chars/4 estimate catches the burst the usage report hasn't seen yet. (Secondary: chars/4 underestimates dense code/JSON; consider chars/3.5 for the estimate used in this guard.)

**50b. Compaction structurally cannot fix doc-heavy overflow.** Both full and micro compaction preserve the most recent `DEFAULT_PRESERVE_RECENT_TURNS = 3` turns **verbatim** (`compaction.rs:266`) — but in an ingestion workload the newest 3 turns *are* the giant ones. Summary replaces the old turns, the recent 100KB+ tool results stay, the compacted history is still over the window, and the next request fails again. When there are no old turns to fold at all, `compact_json_messages_apply_with_summary` returns unchanged → `SkipReason::NoSafeBoundary` → nothing happens.
**Fix:** an emergency degradation tier: when a request still exceeds the window after (or despite) compaction, apply the existing A1 trimming machinery (`cap_tool_output_for_context`) to the **recent** turns with an aggressive emergency budget (e.g. 8k chars/result, marker included) before failing the turn. Correctness beats fidelity once the alternative is death; the full text is already safe in the session store.

**50c. Three failures permanently brick compaction for the subagent.** `autocompact.rs:43` — `MAX_COMPACT_FAILURES = 3` sets `disabled = true` with no reset path. The summary call itself is an LLM call: a rate-limit blip or transient provider error during three summary attempts counts as three "real failures", after which **every** later round skips compaction (`should_attempt` false) and the subagent limps into repeated overflow until it dies.
**Fix:** scope the breaker — transient provider errors (rate-limit, HTTP) get a cooldown (skip N rounds) instead of permanent disable; reserve `disabled` for `InvalidSummary`/structural failures; reset `consecutive_failures` after any successful *request* round, not only successful compaction.

**50d. The reactive overflow path retries exactly once.** `stream_round.rs:238` — on `is_context_window_exceeded`, compact-and-retry happens once per turn (`reactive_overflow_retried`); combined with 50b the retry usually still overflows and the turn fails.
**Fix:** allow the retry to escalate: first retry = compaction; second retry = compaction + emergency recent-turn trimming (50b's tier); then fail.

What is already good and should not be touched: the summary call bounds its own input (320KB pre-trim + trim-oldest retry ×3, `autocompact.rs:232-246`) — the "compaction request itself overflows" failure was anticipated and handled; the fix landed in the campaign. The remaining spiral is trigger timing (50a), the untouchable recent window (50b), and breaker policy (50c/d).

**Relationship to issue #98 (compaction overhaul):** #98 is a strong design (background segment summaries, deterministic ledger + digest-snip, cheap summarizer model, cache-aware swaps) and would resolve 50c/50d *for the main agent* — its stage-3 digest-snip is LLM-free, so a bricked summarizer can no longer mean "no compaction at all". **But it does not fix this finding**, for three reasons: (1) **subagents are explicitly out of scope** ("No subagent compaction in this issue"; the reported symptom is subagents) — and its premise "subagents rely on reactive overflow retries rather than compaction" is stale: `request_round.rs` already does proactive subagent compaction, with the 50a–50d defects; (2) **50a (trigger lag)** is unaddressed — #98's preserve-list keeps `last_known_context_tokens` as the primary signal without the `max(last_known, fresh_estimate)` guard; (3) **50b (recent giant turns)** is unaddressed — every #98 stage, including digest-snip, targets *old* turns; a recent window that alone exceeds the budget (the doc-ingestion case) survives every stage verbatim. **Recommended split:** land 50a (one line) and 50b's emergency recent-turn trimming now as a stopgap on the current machinery — both are independent of the overhaul — and extend #98 (or file a sibling issue) for subagent adoption of the segment machinery, which #98 already places in archon-context for exactly that purpose.

**Implementation verified (commit `8435343f`, 2026-07-26):** all tiers landed — tier 0 ingest cap with head+tail marker and serialized-JSON-aware binary-search fit (`tool_round.rs:198`, `cap_tool_output_to_bytes`), full content still recorded to the raw transcript; tier 1 trigger `last_known.max(fresh)` verbatim (`request_round.rs:298`); tier 2 emergency projection ladder-gated; tier 3 transient/structural breaker split with 30s cooldown (`autocompact_recovery.rs`); tier 4 two-step `RecoveryLadder` with `RecoveryTelemetry`. Watch-items: unclassified overflow errors get no retry (`next_unclassified` → None), and non-Anthropic providers fall back to the 1MB config default rather than a derived per-field limit — confirm 1MB sits under the actual limit of the provider that produced the original 400. Closure evidence (death-record triage, live before/after run, both-sides captures, macOS+Linux gates) still outstanding per the issue's own bar.

**Filed as [issue #103](https://github.com/ste-bah/archon-cli/issues/103)** (sub-issue of #98), in the campaign house style: four-tier stopgap design (burst-aware trigger, emergency recent-turn trimming via A1 machinery, scoped breaker with cooldown, escalating reactive retries), acceptance criteria with both-sides fixture tests, live-evidence closure bar, and explicit non-goals deferring segments/ledger to #98.

## Refactor — split archon-core/src/config.rs

**39. config.rs is a 2,287-line grab-bag and growth hotspot.** ~60 config structs for every subsystem plus loaders, validators, save helpers, and ~230 lines of tests in one file. World-model alone is ~550 lines (11 structs + its validators). The crate already splits config concerns elsewhere (`config_layers.rs`, `config_diff.rs`, `config_source.rs`, `config_watcher.rs`), so this file is the outlier.
**Fix (mechanical, zero behavior change, no call-site churn):** convert to a `config/` directory module. `config/mod.rs` keeps `ConfigError` + root `ArchonConfig` and `pub use`-re-exports every moved type so all existing `archon_core::config::X` paths still resolve. Move by the file's existing section boundaries (line refs are pre-split):

| New file | Contents (current lines) | ~Size |
|---|---|---|
| `models.rs` | `ModelsConfig`, alias maps, `resolve_anthropic_model`/`resolve_codex_model`, `CodexProviderConfig`/spoof/manifest (88–288) | 200 |
| `llm.rs` | `ApiConfig`, `LlmConfig` + OpenAI/Bedrock/Vertex/Local, `IdentityConfig`, `CustomIdentityConfig` (418–637) | 220 |
| `remote.rs` | `SshConfig`, `SshRemoteConfig`, `WsRemoteConfig` (349–417) | 70 |
| `runtime.rs` | `ToolsConfig`, `SubagentConfig`, `PermissionsConfig`, `ContextConfig`, `VoiceConfig`, `WebConfig`, `CostConfig`, `LoggingConfig`, `SessionConfig`, `CheckpointConfig`, `ConsciousnessConfig`, `TuiConfig` (289–348, 638–756, 1617–1741) | 300 |
| `memory.rs` | `MemoryConfig`, `AutoCaptureConfig`, `AutoExtractionConfig` (757–816) | 60 |
| `learning.rs` | `LearningConfig`, `SonaLearningConfig`, `AgentEvolutionConfig`, `ToggleConfig`, `GnnModelConfig`, `GnnTrainingConfig`, `GnnAutoTrainerConfig` (817–852, 1023–1164) | 250 |
| `reasoning_quality.rs` | the 5 `ReasoningQuality*` structs, `SessionBriefingConfig` (853–1022) | 170 |
| `world_model.rs` | all 11 `WorldModel*` structs, `ReflexionConfig`, `validate_world_model_jepa`, `validate_world_model_guardrails` (1165–1616, 1856–1960) | 550 |
| `io.rs` | `default_config_path`, `load_config`/`load_config_from`/`load_config_if_exists`, `write_example_config`, `save_voice_enabled`, `save_world_model_guardrail_modes` (1747–1755, 1961–2052) | 310 |
| `validate.rs` | top-level `validate()` (1756–1855), delegating to per-module validators | 100 |

Rules: keep every serde attribute and `Default` impl with its struct; move the `#[cfg(test)]` tests (2057–2287) to the module whose types they exercise; no field or signature changes in the same PR so the diff reviews as pure motion.
**Do NOT** relocate domain configs into their domain crates (e.g. `WorldModelConfig` → archon-world-model): archon-core and archon-world-model have no dependency edge in either direction (verified in Cargo.toml); the binary bridges them via `src/command/world_model/*`. Adding that edge would drag cozo/JEPA deps into archon-core. Composition-in-core is correct; it just needs submodules.

## Refactor — other files over 500 lines

Measured as **code lines** (excluding inline `#[cfg(test)] mod tests`), because several "big" files are half tests. Repo convention already favors splitting (REM-2 split plans, `event_loop/`/`hooks/` directories, clippy cognitive-complexity gate); target ≤500 code lines for files being touched — don't mass-split untouched cohesive files.

**Tier 1 — split (real seams, active growth):**

| File | Code lines | Split plan |
|---|---|---|
| `archon-docs/src/store.rs` | 1,262 | `store/` directory by relation family: `documents.rs`, `chunks.rs`, `pages.rs`, `images.rs`, `embeddings.rs`, `counts.rs`; `mod.rs` re-exports. |
| `archon-session/src/storage.rs` | 1,027 | `storage/{schema,sessions,messages,tags_names}.rs`. Do together with findings 15/39 (batch-write + count-clamp rewrite touches the same code). |
| `archon-tools/src/agent_tool.rs` | 942 | Two tools in one file: move `AgentCatalogTool` → `agent_catalog_tool.rs`, `classify_failure_prefix` + helpers → `agent_tool/failure.rs`, keep `AgentTool` in place. |
| `archon-core/src/hooks/registry.rs` | 897 | One 700-line `impl HookRegistry` (lines 132–838). Split the impl across `registry/{load.rs,matching.rs,persist.rs}` (multiple `impl` blocks, same type); TOML merge helpers → `persist.rs`. |
| `archon-llm/src/identity.rs` | ~1,000 (tests interleaved mid-file) | Security-sensitive (spoof/identity). `identity/{mode.rs,provider.rs,fingerprint.rs}` matching its three existing section headers; move interleaved test mods to `identity/tests.rs`. |
| `archon-pipeline/src/research/quality.rs` | 903 | ~400 lines are static data tables (`AGENT_MIN_LENGTHS`, `CRITICAL_AGENTS`, `AGENT_EXPECTED_SECTIONS`). `quality/{tables.rs,score.rs}`. |

**Tier 2 — data-table files, different treatment (not a code split):**
`coding/agents.rs` (1,180-line static agent array), `research/agents.rs` (916, same), `archon-core/src/agents/catalog.rs` (~500 code). These are *data*, not logic. Either split per-phase (`coding/agents/phase1.rs`…) or better: move definitions to a TOML/JSON asset embedded via `include_str!` + serde with a startup validation test — agent definitions review better as data than as Rust literals.

**Tier 3 — cheap win, no design work: extract inline test mods to sibling files.**
These files are ≥50% inline tests; the repo already uses the sibling pattern (`agent/tests.rs`, `subagent/tests/`): `src/command/providers.rs` (1,011 total / 591 code), `src/command/copy.rs` (926/442), `src/command/dispatcher.rs` (923/149), `archon-tools/src/send_message.rs` (926/351), `coding/facade.rs` (1,032/562), `research/facade.rs` (999/466), `compression.rs` (1,155/682), `kb/query.rs` (874/550), `src/command/{permissions,effort,garden,context}.rs`. Move `mod tests` to `<name>/tests.rs` (or `#[path]`-included `<name>_tests.rs`) — zero API churn, halves the files.

**Leave alone (big but cohesive or already split):** `runner.rs` (already has 4 submodules), provider impls (`bedrock.rs`, `codex/client.rs`, `providers/registry.rs`), `completion/store.rs` + `verification_gates.rs` (uniform CRUD/gate patterns), `consciousness/assembler.rs`, `hooks/types.rs`, `core/dispatch.rs` (394 code), `skills/agent_skills.rs` (one ~50-line Skill impl per section).

**Special case:** `archon-learning/src/apply.rs` (817 lines) — the problem is a single ~790-line `apply_decision` function, not the file. Decompose the function into per-decision-kind helpers; the file split falls out naturally.

## Deep pass — consciousness, TUI, gametheory, coding pipeline (logic-level)

**40. Corrections reinforce the wrong rule — rich-get-richer loop.** `archon-core/src/agent/memory_integration.rs:212-224`: on every detected user correction it calls `RulesEngine::get_rules_sorted()` and reinforces `rules.first()` — the *highest-scoring* rule, with no relevance matching to the correction (the comment says "top matching rule" but nothing matches). The top rule gains +5 per correction forever, regardless of content.
**Fix:** score rules against the correction text (keyword overlap now; embedding similarity later), reinforce the best match only above a threshold, otherwise skip.

**41. Rules multiply unbounded and all of them go into the prompt.** `archon-consciousness/src/corrections.rs:157-172`: every correction recorded with `rule_id: None` (which is what archon-core always passes) creates a **new** rule `"Avoid: {raw user text}"` — no dedup, so "no, wrong file" becomes a permanent rule each time the crude phrase heuristics fire. `rules.rs:271 format_for_prompt` then renders **all** rules with no cap (the assembler's token budget just mid-truncates the block).
**Fix:** before `add_rule`, search existing CorrectionDerived rules for similar text and reinforce instead of create; cap `format_for_prompt` at top-N (e.g. 10) and drop rules with score < ~5; derive rule text via a template or LLM summarization rather than raw user input.

**42. Rule trend metadata is dead.** `trend:*` tags are written once at `add_rule` (always Stable) and never updated; `rules.rs:219 calculate_trend` ignores them and just thresholds the current score (>60 = "rising").
**Fix:** compute trend by diffing against the persisted `RuleScoreEntry` snapshot from `persistence.rs` (already exported/imported across sessions), or delete the tags and the enum.

**43. recall_corrections truncates unordered results.** `corrections.rs:206` — `search_memories` order is unspecified; `truncate(limit)` keeps arbitrary corrections. Sort by timestamp desc (or severity) before truncating.

**44. TUI streams render at ~4fps and input blocks the runtime.** `archon-tui/src/event_loop/mod.rs:236-296 run_inner`: agent events are drained only once per loop iteration, then the loop blocks in synchronous `crossterm::event::poll` for 250ms (80ms during animation) on the tokio runtime. Token deltas arriving during the poll wait for it to return before being rendered — after fixing the Anthropic fake-streaming (#17), this becomes the visible bottleneck.
**Fix:** replace poll/read with `crossterm::event::EventStream` and a `tokio::select!` over {event_rx.recv(), input stream, animation interval}; redraw when any fires. Removes both the blocking call and the frame-rate cap.

**45. Two parallel TUI event loops.** `run_event_loop` (dispatcher path) and the legacy `run_inner`/`run_tui` path coexist; the new loop no-ops most `TuiEvent` variants and the old one no-ops the new variants (documented as the unfinished TUI-107 migration). Finish the migration and delete the dead arms — this is where subtle event-routing bugs will breed.

**46. CompilationGate blocks the async runtime for a whole build.** `archon-pipeline/src/coding/gates.rs:244` — `async fn run` uses `std::process::Command::output()` synchronously; a `cargo build` can hold a tokio worker for minutes, and there is no timeout.
**Fix:** `tokio::process::Command` + `tokio::time::timeout` (~10 min), kill on expiry.

**47. Test-file detection breaks on Windows paths.** `coding/gates.rs:120 is_test_file` only matches `/test/`, `/tests/` — backslash paths fail, so on Windows the forbidden-pattern gate flags `todo!()`/`stub` inside legitimate test files.
**Fix:** normalize separators (`path.replace('\\', "/")`) before matching.

**48. Orphan gate re-scans the whole project per new file.** `coding/gates.rs:362 find_references` walks and reads every source file once *per* new file — O(new_files × project_files).
**Fix:** single pass — build one combined alternation regex over all stems, walk once, record which stems matched.

**49. Coding quality scorer applies Rust-only heuristics to whole outputs.** `coding/quality.rs` scores the raw agent output: prose/markdown around code gets the magic-number and TODO penalties ("TODO list for next steps" is penalized), and non-Rust code loses documentation/test points for lacking `///` and `#[test]`.
**Fix:** extract fenced code blocks first, detect language per block, and apply language-appropriate pattern sets; score prose separately or not at all.

Clean on inspection this pass: consciousness `assembler.rs` (cache-marker placement is correct and well-reasoned) and `inner_voice.rs`; TUI `output/buffer.rs` (viewport-only rendering with revision/theme/width caches — good design); gametheory facade/quality (advisory gates persisted, defensive fallbacks); archon-completion verification gates; research `quality.rs`/`citation_gate.rs` (data-table-driven heuristics, deterministic).

## Design roadmap — learning, personality, autonomous thought (R1–R8)

The stated goal of the system is an agent that learns as it goes, develops a personality, thinks on its own, and organises its thoughts. The machinery for all four **already exists as crates** — the problem is that the loops are either broken or never wired into the live agent. Ground truth from this audit:

- **Wired and running:** memory extraction (background LLM pass per N turns), memory injection, Memory Garden consolidation at session finish (6 phases: decay, staleness prune, dedup, fragment merge, overflow prune), rule decay 1.0/turn (`turn_completion.rs:56`), rule reinforcement on corrections, inner-voice mood block, first-turn personality briefing.
- **Wired but broken:** reinforcement hits the highest-scoring rule instead of the relevant one (finding 40); every correction mints a duplicate raw-text rule (41); memory recall full-scans (9); KB semantic search unimplemented (6).
- **Built but never wired:** the entire `archon-cognitive` ExecutiveLoop — SituationClassifier → CandidatePlanner → WorldModelScorer → PolicyGate → VerificationEngine → DecisionRecord → LessonSink → SelfModelStore. Grep-verified: zero references outside its own crate/tests; the live agent is a plain LLM tool loop.

**R0 (prerequisite).** Fix findings 9, 11, 17, 40–43 first. A learning system whose retrieval full-scans, whose reinforcement rewards the wrong rule, and whose "corrections" are phrase-heuristic false positives cannot learn — it accumulates noise. Everything below assumes R0.

**R1. Wire the ExecutiveLoop into the live turn — in shadow mode first.**
Run classify→plan→score alongside each real turn without changing behaviour: record a `DecisionRecord` (what the agent chose, what the world-model scorer predicted) and, after the turn, the actual outcome (tool errors, verification results, user correction y/n). This produces the training signal every other item needs, at zero behavioural risk. Graduate to letting `PolicyGate`/`VerificationEngine` gate real actions only once shadow-mode prediction accuracy is measured (R8). Entry point: `archon-core/src/agent` turn pipeline; the cognitive crate's `NoopActionExecutor`/`NoopLessonSink` seats are designed for exactly this substitution.

**R2. Credit assignment: link corrections to their cause.**
`detect_and_record_correction` always passes `rule_id: None`, so corrections never connect to the decision that caused them. When a correction fires: fetch the previous turn's DecisionRecord + tool calls (archon-provenance already models cause edges), run a cheap-model attribution pass ("what went wrong, which action/assumption caused it, what rule follows"), store a structured lesson linked by provenance edge, and reinforce the *matched* rule — or create one only after an embedding-similarity dedup check against existing lessons. This replaces the current behaviour (reinforce top rule + mint noise rule) with an actual learning signal.

**R3. Replace phrase-heuristic correction detection with a classifier.**
The current triggers (`starts_with("no,")`, `contains("i said")`, `starts_with("stop ")` — `memory_integration.rs:148-182`) misfire constantly and every misfire becomes a permanent rule. Run an async haiku-class classification off the critical path returning `{is_correction, type, confidence}`; record only above a confidence threshold. Cost is negligible next to the existing per-10-turn extraction call.

**R4. Extend the Memory Garden into a real "sleep" cycle.**
The garden already prunes and dedups. Add the generative phases that turn episodes into knowledge: (a) cluster related episodic memories by embedding similarity and summarise each cluster into one semantic memory (LLM pass); (b) promote lessons that have been reinforced ≥N times into rules; (c) retire rules not re-triggered in M sessions (decay exists; retirement doesn't — score floor is 0 but the rule still occupies the prompt); (d) run nightly via the existing cron scheduler, not just at session finish. Gate the generative phases through archon-learning's proposal→approve→rollback pipeline so self-modification stays governed.

**R5. Personality as evidence-derived state, not config.**
`PersonalityProfile` is static config; `inner_voice` resets every session; the briefing is boilerplate. Populate the unwired `SelfModelStore` (archon-cognitive) from outcome statistics: per-tool and per-domain success rates, calibrated confidence, user-preference lessons (verbosity, asking-vs-acting) mined from the correction history. Persist `inner_voice.struggles/successes` across sessions instead of discarding them. Generate the first-turn personality/memory briefing *from this state* — "I tend to over-edit configs in this repo; last week's lesson: run tests before claiming done" is personality; a static trait list is not. Bound drift per session (the world-model guardrails crate already implements bounded-change patterns to copy).

**R6. Metacognition worth the name.**
The inner voice is numeric mood (+0.02 confidence per tool success). Replace its prompt block with a periodic private reflection: every N turns or when struggle signals fire (repeated tool failures, correction received), a cheap-model pass over the recent transcript writes 2–3 sentences — "what am I trying to do, is the approach working, what should change" — stored as a memory and injected next turn. The injection plumbing (`to_prompt_block`, compaction snapshot/restore) already exists; only the content generation changes. This is the cheapest path to visible "thinks on its own".

**R7. One mind, not five silos — unify the knowledge substrate.**
Memories, docs KB, pipeline KB, LEANN code index, and world-model state are five disconnected stores with three embedding paths. Build a single retrieval facade (archon-knowledge is the natural home) over the shared embedding provider: one `recall(query)` that returns a ranked, provenance-linked mix of memories, lessons, doc chunks, code, and KB concepts. The KB's concept extraction (`kb/compile.rs`) already builds hierarchy — wire its output into recall (finding 6) and the agent gets "organised thoughts": concepts linking episodes linking sources, traversable via the graph-context machinery that already exists in `kb/query.rs`.

**R8. Measure intelligence or it isn't improving.**
None of the above is verifiable without metrics. Emit via archon-observability and surface in `cognitive_view`/`doctor`: corrections per 100 turns (should trend down), lesson reuse rate (recalled lesson actually cited in output), memory recall precision (injected memories referenced vs ignored), rule churn (creates vs retirements), shadow-mode world-model prediction accuracy (R1). Store per-session in the learning ledger; the first question after every change to R1–R7 is "did the curves move".

Sequencing: R0 → R2+R3 (signal quality) → R1 shadow (signal volume) → R4+R5 (consolidation into identity) → R6 (visible metacognition) → R7 (substrate) — with R8 instrumented from the start.

## TUI output & thinking UX (T1–T7)

Deep-read of the display path (`event_channel.rs`, `output/buffer.rs`, `output/render_cache.rs`, `output/thinking.rs`, `render/body.rs`, `app.rs`). Three of these explain "text goes off the screen / doesn't show everything" — it is not one bug, it is three stacked ones (T1–T3).

**T1. Assistant text is permanently dropped under burst load.** `event_channel.rs:186-205`: the TUI event queue is bounded (1024) and classes `TextDelta`/`ThinkingDelta` as sheddable "Progress" events — when the queue fills, `drop_oldest_progress` **deletes queued TextDeltas, and each one carries a chunk of the assistant's response text**. That text never reaches the OutputBuffer and is unrecoverable in the UI. Two audited findings amplify it: the Anthropic client emits the *entire* response as one delta burst at completion (finding 17 — thousands of sends in milliseconds), and the drain loop sits blocked in `event::poll` for up to 250ms while the queue fills (finding 44).
**Fix:** never shed content-bearing events — **coalesce** instead. In `TuiEventSender::send`, when the tail of the queue is a `TextDelta` and the incoming event is a `TextDelta`, append the string to the queued event (same for `ThinkingDelta`). Bounded queue length, zero content loss. Keep drop-shedding only for stateless progress events (`VideoIngestProgress`). Add a `dropped_content` counter that must stay 0.

**T2. Hard 65,535-row scroll ceiling.** `render_cache.rs:16-21` + `buffer.rs:227,235`: wrap offsets and `total_wrapped` are `u16`, explicitly saturated with `.min(u16::MAX)`; `scroll_offset` is also u16. Once a session transcript exceeds ~65k wrapped rows (a few large tool outputs or a long session), scroll math freezes at the cap: content beyond it cannot be scrolled to, and the auto-scroll bottom position is wrong.
**Fix:** make offsets/total/scroll u32. `Paragraph::scroll()` still takes u16 but `paragraph_scroll_y` is relative to the sliced viewport start (`visible_line_range`) so it stays small — only the global math needs widening.

**T3. Wrap simulation counts chars, not display width.** `buffer.rs:286 count_wrapped_rows` uses `chars().count()`, but the terminal (and ratatui's `Wrap`) measures display cells — emoji/CJK are 2 cells wide. Any wide characters make the simulation underestimate rows, so auto-scroll lands short of the bottom and **the last line(s) sit hidden below the viewport**. Additionally, wrap offsets are computed from `raw_lines` (markdown source) while the Paragraph renders markdown-*styled* lines whose visible text length can differ — same drift.
**Fix:** use `unicode_width::UnicodeWidthStr::width()` in the wrap simulation, and compute offsets from the rendered spans' text (`line_text()` already exists in `render/body.rs:91`) instead of raw markdown.

**T4. Thinking content is discarded by default — the expand toggle shows nothing.** `app.rs:230-237`: deltas are only accumulated when `show_thinking == true`, and it defaults to `false`; `on_turn_complete` (`app.rs:295`) then `reset()`s the block entirely. So out of the box, expanding thinking mid-turn shows an empty block, and after the turn even the summary count is gone. The Claude-Code-style scaffold (collapsed animated header, expand toggle, "Thought for Xs" summary — `thinking.rs`, `app.rs:347`) is genuinely good; it's fed no data.
**Fix (non-intrusive thinking, CC-style):**
(a) Always accumulate, bounded (e.g. 256 KB/turn) — `show_thinking` should gate *display*, not capture.
(b) While active + collapsed, render one extra dim-italic row under the header showing the tail of the current thought (last wrapped line, rolling). That single row is the "Claude Code feel".
(c) On completion, append a collapsed marker line into the transcript (`✻ Thought for 8.2s`, dim) and archive the block text into a per-turn `Vec<ThinkingBlock>` instead of resetting — bind a key (existing `input/dispatch.rs:46` toggle) to expand the most recent marker inline, and a `/thinking` overlay for older blocks.

**T5. Successful tool output has no transcript presence.** `app.rs:241-272`: tool start writes nothing ("don't clutter"), completion writes to the transcript only on failure; successful output lives in `tool_outputs` (`ToolOutputState` with expand/collapse) which renders outside the main transcript flow. Reading back a session, the narrative has holes where the work happened.
**Fix:** append a one-line collapsed entry per tool call to the transcript — `● Bash(cargo test) ✓ 2.1s (312 lines)` — expandable via the existing `toggle_tool_output` to a bounded excerpt. This is the Claude Code pattern: visible narrative, hidden bulk.

**T6. Expanded thinking can eat the whole viewport.** `render/body.rs:82 reserved_thinking_height` caps at `visible_height - 1`, leaving one transcript row.
**Fix:** cap expanded thinking at ~⅓ of viewport height with its own internal scroll.

**T7. Scroll-lock state is nearly invisible.** When the user scrolls up, auto-follow locks (border colour changes — `body.rs:60`) but nothing says new content is arriving.
**Fix:** when `scroll_locked` and new lines have arrived, render a one-line footer hint: `▼ 42 new lines — PageDown/End to follow` (count from `total_wrapped` delta since lock).

Positive notes for the implementer: scrolling controls already exist (PageUp/PageDown, Ctrl+Up/Down, mouse wheel in `event_loop/mouse.rs`); the render/wrap caches are correctly keyed by revision/theme/width; retention is unbounded (nothing is deleted — T1/T2/T3 are why it *appears* lost). Sequencing: T1 with or before PR 2 (streaming), T2+T3 together (same file), T4–T7 as one "thinking & transcript UX" PR after PR 7's event-loop rewrite.

**T8. Stats surfaces — adjudication of the issue #75 A4 review dispute (verified at `audit/core-remediation-2026-07` tip `3661d8a0`).** A reviewer suggested wiring the A4 usage ledger into `archon-tui::screens::session_stats` / having the TUI read archon-learning. Codex pushed back; Codex is **correct on the architecture**:
- `screens/session_stats.rs:102 compute_stats` is dead scaffold — zero production call sites (only its own tests); the screen is unreachable. It also reconstructs token totals by re-parsing every stored message's JSON.
- Live stats correctly flow through `archon_core::agent::SessionStats` forwarded by `src/session/event_forwarder.rs:14` — the right pattern: TUI consumes events, never reads persistence stores.
- Adding an archon-learning dependency to archon-tui would couple the render layer to the persistence stack. Rejected; this boundary is now a rule: **the TUI never reads Cozo/learning stores directly — runtime numbers arrive via forwarded events, historical numbers via command-layer queries passed in as data.**
**A4 landed and verified** (commit `4da6222`, "feat(llm): persist logical call usage", 52 files): the `llm_call_usage` relation in **archon-learning** (correct side of the boundary) keys one row per `request_id` with run/session/turn/round/role/origin attribution, per-field truthful-availability flags (`input_available`, `cache_read_available`, … — no fake zeros), cache creation/read token fields, `effective_denominator`, `terminal_status`, and `checked_add` overflow handling. The TUI boundary held: `archon-tui/Cargo.toml` unchanged (no archon-learning dependency); `session_stats.rs` only gained availability-aware totals over data it already had access to.
**Remaining actions:** (a) live cache-hit display — extend `archon_core::agent::SessionStats` with cached-token fields fed through the existing event forwarder (the ledger has the data now; the TUI still can't see it live); (b) `compute_stats` is still uncalled from production at commit `4da6222` — wire it behind a command that passes data in (reading `llm_call_usage` rows via the command layer, not re-parsed message JSON), or delete it; (c) rename one of the two colliding `SessionStats` types (archon-core's vs the TUI screen's local one) — the collision is what invited the confused review suggestion in the first place.

**Issue #75 remediation — verified landed (A1/A2/A4), with two review notes.**
- **A1 `13ab4fd`** (age-based tool-result trimming): correct by construction — `project_messages_for_request` (`archon-core/src/agent/tool_result_context.rs`) trims old `tool_result` blocks on a *copy* at request-build time (main agent + all three subagent round paths), head/tail split with an explicit marker; the session store is never touched. Two notes for follow-up: (1) budgets are far more conservative than the issue proposed — old results are kept up to 64k chars (shell 24k, subagent 32k) vs the issue's ~2KB head/tail digest, so the win is real but partial; revisit the constants once ledger data shows actual burn. (2) Only string-form `tool_result` content is trimmed — array-of-blocks content returns `None` from `as_str()` and passes through untrimmed; add a branch for block arrays.
- **A2 `c8d89de`** (Anthropic conversation caching): `prompt_cache_conversation` now defaults true (`config/sections.rs:361` — also confirms the config split landed); markers stripped entirely for non-official base URLs (`is_official_messages_url`); `enforce_cache_breakpoint_budget` respects Anthropic's 4-breakpoint cap. One nuance: when over budget it evicts directives in tools→system→messages order, dropping the tools/system markers first. Defensible (a later message breakpoint still caches the whole prefix through it) but it costs intermediate checkpoint granularity after divergence (e.g. post-compaction); prefer evicting the *oldest message* markers first and keeping tools/system pinned.
- **A4 `4da6222`**: verified above.
- **A3 `d4dd40e`** (stable curated tool schemas): verified. General-purpose agent went from `allowed_tools: None` (all ~72 tools) to an explicit `[Read, Grep, Glob, Bash, Write, Edit]` set, with tests asserting the prompt no longer promises "all available tools"/"web searches". MCP model-facing descriptions capped at 1,024 bytes with char-boundary-safe truncation while the raw description stays intact for permission classification — clean separation. Provider-level loopback tests (Anthropic/OpenAI/Bedrock/Codex) assert serialized tools are byte-stable across turns. Notes: (1) `fork` deliberately keeps `allowed_tools: None` ("inherits parent's full tool set") — reasonable for a fork, but it means fork subagents still carry the full schema payload; if fork usage is common, a follow-up could inherit the *parent's effective* subset. (2) Behavior change to watch: general-purpose subagents lose WebFetch/WebSearch/docs tools they previously had — any workflow that relied on a general-purpose child doing web work now needs a different agent type. (3) Their own record notes the global baseline gate is blocked by three archon-pipeline `llm_adapter` test removals from the A4 refactor — needs reconciling before closure.
- **Status update (2026-07-25):** A1–A4 and B1–B4 all shipped and now **all independently verified in code**. B-series: `22d6517` restructures the workflow agent prompt into a run-stable prefix (constraints + task universe + rules + result schema) with per-call fields (`call_id`, input) *after* it (B1) and switches to `compact_json` (B4); `f9eb77d` adds `task_universe_digest`/`task_contract_digest` for reducers (B3) and `digest_wave_evidence`/`digest_old_records` keeping the latest wave full and older waves as outcome digests (B2), with 382 lines of prompt-growth tests asserting bounded O(N) payloads; `c4d8f8c` adds 341 lines of wire-stability tests plus 221 lines of learning-store fidelity tests (the "digest at prompt boundary, never persistence boundary" contract). **Issue #75 stays open.** Remaining closure blockers, none of them code: (1) real vLLM before/after time-to-first-token benchmark; (2) macOS validation; (3) same-canary before/after aggregate ledger totals; (4) final whole-evidence adversarial review once those measurements exist. Do not close on implementation completeness — the issue's own bar is measured evidence, and the remaining items are exactly the measurements.

**Tracker status (checked 2026-08-01 evening, tip `327f0b7fb`):** the Windows-parity wave predicted by findings 21/23/47 is being burned down by the now-active Windows CI: workflow verifiers no longer hardcode `sh` (`2dae7a43e` extends the `06a5594` shared resolver; `327f0b7fb` feeds scripts via stdin), Windows absolute paths recognized in evidence extraction (`b8d211d36`), orphan-gate display paths and session file locks fixed (`ea1b0fa0d`), Cozo verbatim `\\?\` prefix stripped from canonical paths (`6ce5d720c`), and a Windows escape-guard security hole closed (`fcc7cc8cd`) — 8+ real Windows bugs surfaced by CI in a week, none of which were reachable before #92. Also: nextest hard per-test timeouts (`43ee10ca8`, "hangs are named, not silent"), docs TUI routing + `docs delete` with content-hash freeing, spreadsheet ingestion, and Opus 5/Sonnet 5/GPT-5.6 model catalog entries.

**Tracker status (checked 2026-08-01 morning):** verified at branch tip `e061cd1fa`, v1.4.0 released.
- **Catalog #108/#109 — landed, exceeds the approved design.** `catalog_state.rs` now has `staging: Mutex<ImmutableCatalogSnapshot>` (serialized writers), `ArcSwap` publication of complete snapshots after every mutation, and the batch path ("insert entries under one writer lock and publish once for bulk discovery") with accepted/rejected record reporting. The DashMap→immutable representation swap became its own gated migration (#109) decided by **benchmarks**, not assertion — representation comparisons committed as evidence.
- **Windows CI (#92, closing the report's findings-21/23/47 regression gap) — landed.** `ci.yml` matrix now `[ubuntu, macos, windows-latest]` with a Windows dependency installer and nextest; enabling it immediately caught and fixed a real Windows-only broken read loop (`a0a4f7ba7`) — the exact silent-regression class the report warned about.
- **W1 (JEPA degenerate inference inputs) — core defect closed.** `synthetic_runtime_window` is gone from the non-test inference path; prediction now builds real trace windows via the embedding adapter and window builder (`predict/01_inference.rs`), and the encoder is trained on real embeddings with real capacity (`659a0c579`…`05fd9f292`). Re-measure `latent_surprise` against baseline per the PR-8 verification note.
- Also landed: config-drift CI gate (`55c9b64f6`), cross-platform setup installers (Amazon Linux 2023, rustup, Windows deps), plugin-collection licensing docs.

**Tracker status (checked 2026-07-26):** this report's findings were converted into a GitHub issue campaign. Closed: #52–#58 (doc-vector fixes, search stack = findings 1–5, Anthropic streaming = 17–18, correction/rule feedback = 40–43), #59–#64 (consciousness/memory integrity, Cozo concurrency = 11–13), #65–#67 (W1 inference inputs, guardrail correlation, verified turn finalization = W2), #68–#73 (W4 compile fix, HNSW snapshot naming, stalled-stream cancel, CI hygiene, baseline gate, TaskStop cancel). Open: #74 (mechanical gate debt), #75 (evidence blockers above), and a new evidence-verification wave #86–#97 (R0 evidence, T1, T2–T3, T4–T7, runtime evidence for findings 1–16, finding 39 tier-2, Windows CI for 21/23/47, TUI fairness/orphan scan 44/48, 45/46/49, W1–W4 live evidence, cognitive-loop capability claims, 19/36) plus #98–#100 (compaction overhaul, Vertex Gemini tool schemas, MCP snapshot naming).

## World model / JEPA assessment (W1–W6)

Deep-read separately (it was sweep-level in the first pass). Verdict: **this is the best-engineered subsystem in the codebase** — typed trace schema (state/action/next-state embeddings + deterministic outcome labels), JEPA with CPU/candle-CUDA/MLX-Metal backends and parity gates, checkpoint registry with promotion gates, cold-start gating, per-call latency budget enforcement, latent-surprise measurement on outcomes, shadow/counterfactual scaffolding. And unlike the cognitive crate, it **is wired**: guardrail runs on every interactive turn (`session_loop/prompt_turn.rs:46`), on pipeline/code/research runs, and the auto-trainer tick is scheduled after runs and via the cognitive daemon.

**Will it prevent mistakes today? Only marginally** — because of two gaps:

**W1. Degenerate inference inputs — the model never sees real state.** `src/command/world_model/predict/01_inference.rs:24-25`: at runtime, `state = embed(prompt_summary)` and `action = embed("action=" + the same prompt_summary)` — state and action are near-identical vectors; the JEPA path uses a `synthetic_runtime_window` built from the same text. Training traces contain real state→action→next-state transitions from ingested activity, so there is a severe train/serve skew: predictions reduce to "what usually happens after prompts that sound like this."
**Fix:** build the runtime `TraceWindow` from the live session's actual recent transitions — the activity events already captured (tool calls, errors, retries, files touched) via the existing `TraceWindowBuilder` (`representation.rs`), and embed the *candidate action* (tool name + input summary), not the prompt echo. This single change is worth more than any model improvement.

**W2. The guardrail cannot actually stop anything.** In the interactive path the decision only prints a TUI warning (`prompt_turn.rs:51`) and is recorded; `allowed_to_finalize=false` affects the *outcome record's status* (`BlockedMissingVerification`), not agent behaviour. Nothing prevents the agent from claiming "done" without running the required verifications.
**Fix:** connect the two crates that already exist for this: inject `decision.required_actions` into the turn as a system reminder ("guardrail requires: run tests before claiming completion"), and gate completion through archon-completion's verification gates — `WorldGuardrailDecision.required_actions` → `CompletionClaim` requirements → `TestsPassGate`/`BuildPassesGate` must pass before the completion claim is accepted. Guardrail predicts, completion-engine enforces.

**W3. Task classification is keyword matching.** `guardrail/01_decision.rs:259 classify_task` — `contains("bug")` → Debugging, `contains("delete")` → ExternalSideEffect. Risk tier and required verifications all flow from this. **Fix:** reclassify after the first tool call from the *planned actions* (tool names are far more reliable than prompt words), or use a cheap-model classifier (shares infrastructure with roadmap R3).

**W4. Guarded at turn granularity, not action granularity.** The `ToolRun` surface and `ShellCommand` action kind exist, but tool dispatch never calls `begin_guarded_action` — the place where a failure prediction is most valuable (before a risky edit or destructive command) is unguarded. **Fix:** hook the ToolRun surface into `ToolRegistry::dispatch` for Risky/Dangerous-classified calls only; the latency budget enforcement (`max_guardrail_overhead_ms`) already exists to keep this cheap.

**W5. Label quality caps model quality.** `user_correction` labels come from the broken phrase-heuristic detector (finding 41/R3) and `success` is sparsely set. The model learns from these. **Fix:** R3 (classifier-based corrections) directly improves world-model training data; additionally, use archon-completion verification results (test/build outcomes) as ground-truth `success` labels — they're deterministic and already recorded.

**W6. Use latent surprise, don't just store it.** `latent_surprise` (predicted-vs-actual embedding distance) is computed per outcome and persisted, then unused. **Fix:** (a) weight training examples by surprise (prioritized replay — surprising transitions teach most); (b) surface high-surprise moments to the reflection pass (R6) — "this went differently than I expected" is exactly what metacognition should chew on; (c) add mean surprise to the R8 metrics as the world model's accuracy curve.

Platform note: MLX backend is Apple-silicon-only, candle-CUDA needs CUDA; the CPU backend keeps training functional on Windows, just slow — acceptable, no action needed.

Sequencing with the R-series: W1 → W3 → W2+W4 (enforcement once predictions are credible) → W5 alongside R3 → W6 alongside R6/R8.

## Coverage appendix

Deep-read: archon-cozo, archon-docs, archon-leann, archon-knowledge, archon-memory (search/embedding), archon-llm (client/retry), archon-core (dispatch/hooks/config/agent), archon-session (storage/listing/checkpoint), archon-mcp (bridge/reconnect), archon-tools (bash/webfetch/docs), archon-pipeline (kb, runner, memory, gnn auto-trainer, coding gates/quality, gametheory facade/quality, research quality/citation gate), archon-permissions (classifier), archon-observability (activity), archon-context (compact), archon-video (asr), archon-sdk (web auth), archon-cognitive (executive loop), archon-consciousness (rules/corrections/assembler/inner_voice), archon-tui (event loop, output buffer, task dispatch), archon-completion (verification gates).
Sweep-level (grep hazard patterns + structure check, no per-line read; nothing flagged): archon-policy, archon-plugin, archon-provenance, archon-meaning, archon-constellation, archon-reasoning-quality, archon-bench, remaining TUI view/render files, research chapters/final-assembly, gametheory specialists/persistence internals.
Excluded per instruction: archon-workflow, archon-trading. Test-support crates skipped.

**Intentionally NOT flagged (do not "fix"):**
- Lock-poisoning `.unwrap()`/`.expect()` on mutexes and `expect("valid regex")` on static patterns — idiomatic, fine.
- The permission classifier being heuristic — perfect shell classification is impossible; unknown-defaults-to-Risky is the right posture. Only the specific `find` gap (25) is actionable.
- Cozo idempotent-DDL via `"already exists"` string matching — ugly but correct until the shared `ensure_schema_once` helper (cross-cutting rec 1) replaces it wholesale.
- `unsafe impl Sync` test-support mocks, `set_var` in tests, `block_on` in benches/tests.
- The research/coding quality scorers being regex heuristics — by design (deterministic, no LLM cost); only the prose-contamination issue (49) is actionable.
- SSE reconnect resetting the retry counter after each successful connect (infinite reconnect for a flapping server) — standard SSE semantics, intentional.

## Cross-cutting recommendations (for the implementer)

1. **Shared Cozo helpers:** most correctness/perf issues repeat the same three patterns — (a) filter on non-key column = full scan, (b) one `run_script` per row, (c) `already exists`-string idempotent DDL. Add to `archon-cozo`: `put_rows_batch(db, relation, rows)` (single multi-row script), `ensure_schema_once(db, ddl)` (memoized per (db-path, ddl-hash)), and secondary-index helpers. Migrate leann/session/kb/learning to them.
2. **Shared text truncation helper** (char-boundary safe) in a util crate; replace all byte-slice truncations (grep `[\.\.` on Strings).
3. **Windows parity:** hooks (`sh`), Bash tool (`/bin/bash`), env passthrough — one `shell_resolver` module used by both.
4. **Blocking-in-async policy:** all embedding, RocksDB, and Cozo work belongs under `spawn_blocking`; add a clippy-style CI grep or wrapper API so new call sites can't regress.
