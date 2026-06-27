# Plan — Wire `archon-ingest-ext` Token-Aware Chunker Into the Live `archon kb ingest` Pipeline

- **Status:** Scoped / not started — awaiting go on PR1.
- **Date:** 2026-06-27
- **Repo:** `ste-bah/archon-cli` (fork `ProfessorVR/archon-cli`, branch `ports-lanham-ingest`, PR #47).
- **Context:** `archon-ingest-ext` landed in PR #47 as a tested, standalone crate. It is **not** consumed by `archon-docs` — this plan scopes that wire-in.
- **Method:** Multi-agent `Workflow` (understand → synthesize → critique → finalize); anchors source-verified.

> **Cross-cutting blocker resolved since drafting:** the Port C spec the code comments reference (`plans/archon-ingestion-ports-spec.md §2`) is **absent from the archon repo but present in the god-agent project** at `plans/archon-ingestion-ports-spec.md`. It is the authoritative source for §4's parity gate and the cross-page `page_end` divergence; import it into the archon repo at PR3 time. `markdown_chunker.py` + the golden corpus still need locating in the god-agent repo.

---

# Wire-In Scoping: archon-ingest-ext Token-Aware Chunker → Live archon-docs Ingest

## 0. Provenance & key corrections

This plan was produced by a 5-reader code-mapping pass over the cloned repo, synthesized, then adversarially reviewed; every `file:line` anchor below was verified against source. The headline correction surfaced by review: the MVP **cannot cap chunk size** (paragraph-splitting does not bound block size; `HARD_MAX` is unimplemented), so its win is **upward merge only**, not full token-budget control. The orphaned-vector / re-embed cost is relocated from "PR3 default-flip" to "**whenever the flag is toggled on a populated store**," the document-level dedup blocker is added to the migration plan, and the switch is re-specified as an **injectable parameter** so the dual-run test is expressible in one process. Changed claims are flagged inline.

## 1. The core mismatch

The new chunker is **structure-first**: `chunk::chunk_blocks(&[Block], min, max, _hard) -> Vec<ChunkOut>` (`crates/archon-ingest-ext/src/chunk.rs:129`) consumes a stream of typed, spatially-anchored `Block { block_type: BlockType, text: String, bbox: [f32;4], page: u32 }` (`chunk.rs:29`) and emits `ChunkOut { text, page_start, page_end, bboxes: Vec<PageBoxes> }` (`chunk.rs:46`) where each `PageBoxes` carries a per-page super-box plus member boxes. The live chunker is **text-first**: `chunking::chunk_with_page_anchors(text: &str, page_offsets: &[PageOffset]) -> Vec<PageChunk>` (`crates/archon-docs/src/chunking.rs:21`) splits flat `full_text` on `\n\n`, trims each fragment (`raw.trim()`, `chunking.rs:32`), flushes at ≥200 bytes, maps byte offsets to pages via `page_for_offset` (`chunking.rs:114`), and emits `PageChunk { content, page_start, page_end }` (`chunking.rs:11`) with **no block type and no bbox**. The only upstream producer in the live path, `OcrExtractResult { full_text, page_count, page_offsets, processing_duration_ms }` (`crates/archon-docs/src/ocr/provider.rs:27`), and the PDF path's `PdfExtractResult` (`crates/archon-docs/src/pdf.rs:14`), both emit flat text from `pdftotext -layout` (`pdf.rs:138`) — **no Marker tree, no block list, no text coordinates exist anywhere in the live pipeline**. So the wire-in is not a drop-in: the new chunker's required input type is produced by nothing live today, and its richest output field (`bboxes`) has no persistence sink.

## 2. Integration strategy

**Recommendation: PR1 = option (b), the synthesized-Block adapter MVP, behind a runtime switch.** Then PR2 = option (a) real Marker-JSON routing; PR3 = flip the default + parity gate.

Justification, grounded in blockers the readers surfaced:

- **Real Marker JSON is produced nowhere in this clone.** The only Marker consumer is `marker.rs` itself; the cited producers `scripts/archon_marker_sidecar.py` / `scripts/chunk_parity_check.py` **do not exist**. There is no `ARCHON_MARKER*` env var, no binary-path resolution, no sidecar client. Standing up real Marker = net-new subprocess + GPU device-autodetect plumbing analogous to `pdf.rs`'s `pdftotext` shell-out — large, and it blocks any spatial/`doc_chunk_spatial` provenance. Gating PR1 on that is wrong sequencing.
- **Option (b) is cheap and captures *part of* the real win now.** Synthesizing `Block`s from the existing `full_text` + `Vec<PageOffset>` lets us swap to the token-aware/heading-aware chunker (`TARGET_MIN=800` / `TARGET_MAX=1200` tokens via code-points/4, `chunk.rs:12-14,57`) at the **single chokepoint** `ingest_artifacts.rs:33` plus the three sibling call sites, with no schema migration and no new external dependency beyond the crate edge.
- **What defers to a later PR:** (i) real Marker sidecar + Marker-JSON routing → real `BlockType::SectionHeader`/`Table`/`Caption` and real bboxes (PR2); (ii) the `doc_chunk_spatial` relation + schema migration + `store::insert_*` to persist `ChunkOut.bboxes` (PR2, only meaningful once real bboxes exist); (iii) `layout::extract_locators` (`layout.rs:50`) Bekker/page-locator capture, orphaned today and needing its own sink (deferred, out of scope).

### 2.1 MVP limitations — carried as hard caveats, not papered over

**(a) No structural logic fires.** Synthesized blocks are all `BlockType::Text` with degenerate `[0,0,0,0]` bboxes, so the heading-boundary flush (`chunk.rs:138-142`, triggered by `SectionHeader`) and any table gate never fire. The MVP exercises **only** the token-budget flush + pairwise undersized-merge. This is acceptable for a chunk-quality upgrade but means the MVP is **not** the final shape.

**(b) The MVP cannot bound chunk size — `HARD_MAX` is inert (corrected claim).** `chunk_blocks` takes `_hard` and ignores it (`chunk.rs:129`); the doc-comment (`chunk.rs:126-128`) calls the oversized-paragraph split "reserved" and it is unimplemented. The accumulator flush at `chunk.rs:140-142` runs *before* a new block is added, so a single block already larger than `TARGET_MAX` is emitted **whole** and never sub-split. **Splitting synthesized text on `\n\n` does NOT bound block size:** `pdftotext -layout` (`pdf.rs:138`) frequently emits a whole page with no blank-line separators, yielding one synthesized `Block` far exceeding `TARGET_MAX` (~4800 chars). Therefore:
  - the headline "800–1200-token chunks" win is realized **only as an upward merge** (`merge_undersized` of small fragments) — there is **no downward split**;
  - an unsplit oversized chunk can exceed the embedding provider's max input-token window, producing `embedding_status='failed'` (indexing failure path) and silent retrieval gaps (see §6).
  - **Required decision for PR1:** either (i) implement an oversized-block fallback **in the adapter** (`synthesize_blocks` hard-splits any paragraph whose `est_tokens` exceeds `TARGET_MAX` at code-point granularity before handing blocks to `chunk_blocks`), or (ii) explicitly accept the ceiling and document it as a known regression. Recommendation: **do (i)** — it is a few lines in the adapter, keeps the size guarantee out of the shared crate, and removes the silent-failure risk. PR2's real `HARD_MAX` path then supersedes it.

## 3. Workstream breakdown

Dependency order is top-to-bottom; each task names real paths.

**Task 1 — Cargo dependency edge (anchor corrected).**
File: `crates/archon-docs/Cargo.toml`. The dependency block ends at line 33, `tempfile.workspace = true` (which sits *after* `archon-cozo` at line 32), followed by the `[dev-dependencies]` block (`serial_test`, `wiremock`) at line 35. Insert the path dep among the `[dependencies]` (before line 35):
```toml
archon-ingest-ext = { path = "../archon-ingest-ext" }
```
No change to the workspace `members = ["crates/*"]` glob (already a member). Blocker check: `archon-ingest-ext/Cargo.toml` deps are only `serde_json` + `regex`, no cycle risk.
**Also:** regenerate and commit `Cargo.lock` with the new edge — CI builds from the locked graph.

**Task 2 — Block-synthesis adapter (new module).**
File: new `crates/archon-docs/src/chunk_adapter.rs` (keep `chunking.rs` under the 500-line FileSizeGuard, `scripts/check-file-sizes.sh`). Add:
```rust
pub(crate) fn synthesize_blocks(text: &str, page_offsets: &[PageOffset])
    -> Vec<archon_ingest_ext::chunk::Block>;
```
Algorithm: for each `PageOffset { page, char_start, char_end }`, slice `text[char_start..char_end]`, split on `\n\n`, **`trim()` each fragment to match legacy `raw.trim()` (`chunking.rs:32`)**, and emit one `Block { block_type: BlockType::Text, text: para, bbox: [0.0;4], page }` per non-empty trimmed paragraph. Specify explicitly:
- **Empty pages / pages with no `\n\n`:** a page lacking blank-line separators yields exactly one block (the whole page) — this is the oversized case from §2.1(b); the oversized-fallback split (Task 2b) must run here.
- **UTF-8 byte-boundary guard (live gap):** `PageOffset` fields are named `char_*` but used as **byte** offsets (`chunking.rs:116`). A naive `text[char_start..char_end]` on a multi-byte boundary panics. Guard by snapping to `char_indices` boundaries (or `str::get(..).unwrap_or_else(...)` with a boundary walk) before slicing.

**Task 2b — Oversized-block fallback (the §2.1(b) decision, in the adapter).**
Same module. After paragraph split, any block with `est_tokens(text) > TARGET_MAX` is hard-split at code-point granularity into `<= TARGET_MAX` sub-blocks (same `page`, same placeholder bbox). This restores the size ceiling that `chunk_blocks` does not enforce, and is the difference between "shipped MVP" and "silent embedding failures on dense pages."

**Task 3 — ChunkOut → PageChunk adapter (field-by-field).**
Same file. Add:
```rust
pub(crate) fn chunkout_to_pagechunks(outs: Vec<archon_ingest_ext::chunk::ChunkOut>)
    -> Vec<crate::chunking::PageChunk>;
```
Field mapping (the asymmetric superset delta):
| `ChunkOut` (`chunk.rs:46`) | `PageChunk` (`chunking.rs:11`) | note |
|---|---|---|
| `text: String` | `content: String` | **rename** (same role) |
| `page_start: u32` | `page_start: u32` | identical |
| `page_end: u32` | `page_end: u32` | identical |
| `bboxes: Vec<PageBoxes>` | — | **DROPPED on the floor** in MVP |

`bboxes` is discarded because (i) MVP bboxes are placeholder `[0,0,0,0]`, and (ii) there is no sink: `ChunkArtifact` (`models.rs:104`) has no bbox column and `doc_chunk_spatial` exists only in doc-comments (`chunk.rs:10,44`; `marker.rs:6`), never `:created`/`:put` in `schema.rs`. PR2 adds the sink.

**Returning `Vec<PageChunk>` is the load-bearing contract (underspecification fixed).** The four call sites split into two builder shapes: two build via `build_chunk_artifacts` (None-prefix path, `chunking.rs:92`), and two construct `ChunkArtifact` **inline** (`ingest_artifacts.rs:36-50` Some-branch; `ingest_multimodal.rs:127-138`). The adapter therefore must emit `Vec<PageChunk>` so it slots in front of **both** builders unchanged — it replaces only the chunk-production step, never the artifact construction.

**Task 4 — Injectable switch at the chokepoint (mechanism corrected).**
File: `crates/archon-docs/src/ingest_artifacts.rs:33`. The selector is **not** a Cargo feature (compile-time, cannot be toggled within one test process) and **not** a process-global env read (makes the in-process dual-run test of §5 fragile). It is an **injectable parameter** — a field on the ingest policy/config struct threaded to `persist_text_artifact_chunks`, e.g. `IngestPolicy { chunker: ChunkerKind }` with `ChunkerKind::{Legacy, TokenAware}`. An env var (`ARCHON_CHUNKER=token_aware`) may *seed the default* of that field at startup, but the function takes the value as an argument so a test can pass both variants in one process. Replace the unconditional call:
```rust
let page_chunks = match policy.chunker {
    ChunkerKind::TokenAware => {
        let blocks = crate::chunk_adapter::synthesize_blocks(text, page_offsets); // incl. Task 2b fallback
        crate::chunk_adapter::chunkout_to_pagechunks(
            archon_ingest_ext::chunk::chunk_blocks_default(&blocks))
    }
    ChunkerKind::Legacy => chunk_with_page_anchors(text, page_offsets), // default
};
```
Default = `Legacy` for safe rollback. Because this is the shared helper `persist_text_artifact_chunks`, this single edit covers call site A's sibling and **both PDF paths** (caller C1 `ingest_pdf.rs:73`, caller C2 `pdf_image_enrichment.rs:362`).

**Task 5 — Thread the same `ChunkerKind` through the other 3 direct call sites** so the switch governs the whole stack uniformly: `ingest.rs:419` (OCR/text-file), `ingest_text.rs:148` (URL/raw-text, synthetic single `PageOffset`), `ingest_multimodal.rs:125` (VLM caption, synthetic single `PageOffset`). For the synthetic-single-page sites the adapter still works (one page, paragraph-split). The selector lives on the policy struct already in scope, so no four copies of branch logic — each site passes `policy.chunker` to the shared helper.

## 4. The S-1 parity gate

To flip the default (PR3) the S-1 gate must be **authoritative and green**: the Rust `chunk_blocks` output is byte-for-byte reproducible against the Python reference on a real golden corpus.

**Present today:** two self-asserting unit tests with hand-transcribed literals — `chunk_parity_matches_python_reference` (`marker.rs:273`) and `parses_sidecar_selftest_fixture` (`marker.rs:318`) — over one synthetic fixture `crates/archon-ingest-ext/tests/fixtures/marker_selftest.json` (a 2-page Document). The `chunk_blocks` constants/flush/merge logic is unit-tested (9 tests, `chunk.rs:227-352`).

**Missing today (blockers):**
- The reference `markdown_chunker.py` — **absent**; zero project `.py` files exist.
- The parity tooling `scripts/chunk_parity_check.py` and the producer `scripts/archon_marker_sidecar.py` — **absent**.
- The spec `plans/archon-ingestion-ports-spec.md §2 (Port C)` — **absent** (no `plans/` dir).
- A real golden corpus of Marker JSON dumps + captured reference chunk outputs — **absent** (only the one synthetic fixture).
- A **known ungated divergence**: `marker.rs:276-278` documents a cross-page max-flush `page_end` correction that diverges from the reference; the parity test deliberately uses only the "clean case."

**Acceptance criterion for the divergence (underspecification fixed).** "Reconcile" is currently undefined and the only authority — `plans/archon-ingestion-ports-spec.md §2` — is absent. PR3 must first **import that spec**, then choose exactly one of: (i) **fix-to-match-Python** (delete the `page_end` correction so Rust == reference), or (ii) **document-as-intentional** (record the correction as a deliberate Rust improvement, update the Python reference *and* the golden corpus to match, and annotate the parity harness to expect it). The gate passes only when the chosen branch is encoded in `scripts/chunk_parity_check.py` and the golden corpus, with zero unexplained diffs.

**To make S-1 authoritative:** import `markdown_chunker.py` + a captured golden corpus from the god-agent repo (open question — confirm canonical location), stand up `scripts/chunk_parity_check.py` as a CI differential harness, and resolve the divergence per the criterion above. **Assumption for PR1:** because the MVP synthesizes all-`Text` blocks, it never exercises `SectionHeader`/`Table`/cross-page-max-flush paths, so PR1 ships behind the switch **without** a live differential harness — S-1 gates only the PR3 default-flip, not the flagged MVP.

## 5. Test plan

- **Unit — adapter (new):** `synthesize_blocks` produces one `Text` block per *trimmed* paragraph with correct `page`; an empty page yields zero blocks; a page with no `\n\n` yields one block; a multi-byte UTF-8 page slice does not panic (boundary guard); **Task 2b: a >`TARGET_MAX` synthesized paragraph is split into `<= TARGET_MAX` sub-blocks**; `chunkout_to_pagechunks` round-trips `text→content` and preserves `page_start/page_end`; empty input → empty output.
- **Unit — chunker (exists, keep):** `chunk.rs:227-352` (9 tests) and `marker.rs` golden tests (`:273`, `:318`) unchanged.
- **Integration — ingest equivalence (assertion scoped, false-fail fixed):** drive `persist_text_artifact_chunks` (`ingest_artifacts.rs:12`) twice on the same input — `ChunkerKind::Legacy` vs `TokenAware`, both passed in **one process** (only possible because the switch is an injectable parameter, Task 4) — asserting both produce valid `ChunkArtifact` rows persisted via `store::insert_chunk` (`store.rs:411`) into `doc_chunks`. **The "fewer/larger chunks" assertion is gated to large multi-paragraph inputs only.** For the three synthetic single-page sites (`ingest_text.rs:148`, `pdf_image_enrichment` C2, `ingest_multimodal.rs:125`) with short content, both paths emit one chunk, so the count-reduction assertion must be skipped for that fixture class or it will false-fail.
- **Integration — provenance/citation invariants:** extend `crates/archon-provenance/tests/provenance_engine.rs:43-95` (hardcodes the 9-column `doc_chunks` shape) to confirm `page_start..=page_end` edge synthesis (`traverse.rs:125-156`) and `answer.rs` `Citation` (`answer.rs:33`) still hold under new boundaries.
- **CI scope (verified, stated):** the new module stays under the 500-line FileSizeGuard (`scripts/check-file-sizes.sh`). The other two blocking jobs are **confirmed unaffected** and must stay green: `scripts/lint/arch-lint.sh` (TUI input-handler async rules — no TUI touch here) and the preserve-invariants `mcp__memorygraph__` leak detection (no graph mutation here). Naming all three so the PR cannot claim CI coverage from `check-file-sizes` alone.
- **Parity (deferred to PR3):** wire `scripts/chunk_parity_check.py` + golden corpus into `.github/workflows/ci.yml` nextest job once the Python reference is imported.

## 6. Risk & blast radius

Boundary changes are the dominant risk, and — corrected — they are **not confined to the PR3 default-flip**. They land **whenever `ChunkerKind::TokenAware` is selected on a populated store**, including a PR1 opt-in toggle.

- **chunk_id churn + orphaned vectors.** `chunk_id = "chunk-{document_id}-{i}"` is **positional** (`chunking.rs:101`). New token budgets change chunk **count and boundaries**, so the id set changes. Stale rows in Cozo `vec_text_chunks` (`schema.rs:261`) and RocksDB `vec/{provider}/{chunk_id}` (`vector_store.rs`, `hnsw_id = blake3(chunk_id)`) are **orphaned** — `prune_orphaned_queue_rows` (`index_queue.rs:177`) cleans only `doc_index_queue`, not the vector stores, and `vector_migration.rs` migrates Cozo→RocksDB but does **not** GC stale `chunk_id`s.
- **Rollback is NOT vector-store-neutral (new risk).** Presenting `Legacy` default as "clean rollback" is true only for code, not data. Flipping the switch **OFF after it ran ON** regenerates legacy positional `chunk_id`s and orphans the token-aware vectors created while it was on — a **second** orphan set, again with no GC. There is no symmetric, side-effect-free toggle at the data layer.
- **Embedding-cache misses → full re-embed.** `content_hash = sha256_str(content)` (`chunking.rs:108`); new boundaries → new hashes → misses in `vec_text_embedding_cache` (`schema.rs:289`) and RocksDB `cache/{provider}/{content_hash}` → mass re-embedding cost.
- **Three boundary regimes ⇒ up to three re-embeds (new).** Boundaries differ across legacy (~200-byte) → MVP (all-`Text` token merge, PR1) → real-Marker (real `BlockType` + bbox, PR2). PR2's real boundaries will **not** equal the MVP's, so adopting PR2 churns `chunk_id`/`content_hash` **again** and forces another re-embed + orphan-GC. Re-embed is **not** a one-time PR3 cost; budget for it at each regime transition.
- **Embedding token-limit ceiling → silent quality regression (new, ties to §2.1(b)).** Without Task 2b, an unsplit oversized chunk can exceed the provider's max input tokens, producing `embedding_status='failed'` and missing retrieval coverage **with no hard error** — directly relevant to the quotation-fidelity expectations in MEMORY. Task 2b is the mitigation; if deferred, this is a documented known regression.
- **Retrieval/citation drift.** `page_start/page_end` flow into `SearchResult` (`retrieval.rs:24`), `Citation` (`answer.rs:33`), and per-page provenance edges (`traverse.rs:147`). The new chunker **preserves both fields**, but coarser chunks shift which page-range each citation reports.
- **Quotation semantics.** `ChunkOut.text` joins blocks with `\n\n`; for the MVP this matches the legacy `\n\n` paragraph join, so content text is comparable (real-Marker `[TABLE] …` rendering is a PR2 concern).

### 6.1 Migration plan (now includes the dedup blocker)

The reindex/wipe migration is required **whenever `TokenAware` is first selected on a non-empty store** (PR1 opt-in or PR3 flip), not only at default-flip. It has **two** steps, and step (b) is mandatory:

**(a) Purge stale vectors/cache:** delete orphaned `vec_text_chunks` + RocksDB `vec/{provider}/` and `cache/{provider}/` entries for affected documents (no GC exists today).

**(b) Force re-chunking past document-level dedup (new blocker).** Purging vectors alone is **insufficient**: re-ingest is a no-op on unchanged content because `ingest.rs:155` `hash_exists_in_sources` and `ingest_text`'s `get_doc_by_hash` short-circuit and return the existing `document_id` on identical `content_hash`. The old `doc_chunks` rows therefore survive and never get re-chunked. The migration must route through `reprocess.rs` (`reprocess.rs:66`) — or delete the affected documents first and re-ingest — so the dedup guard is bypassed and chunks are regenerated under the new chunker.

## 7. Effort & sequencing

- **PR1 — MVP chunker behind an injectable switch (S–M).** Tasks 1–5: Cargo edge + `Cargo.lock`, `chunk_adapter.rs` (`synthesize_blocks` + **Task 2b oversized-fallback** + `chunkout_to_pagechunks`), `ChunkerKind` selector threaded through `ingest_artifacts.rs:33` + the 3 sibling sites, adapter unit tests + scoped dual-run integration test. No schema change. Default = `Legacy`. Opt-in on a populated store triggers §6.1's migration. **Low–medium.**
- **PR2 — Real Marker routing + spatial sink (L).** Net-new Marker sidecar client (subprocess → `parse_marker_str`, `marker.rs:147`) with `ARCHON_MARKER*` config + device autodetect; route real `Vec<Block>` (real `BlockType` + bboxes) through `chunk_blocks_default`; **implement the real `HARD_MAX` oversized split in `chunk_blocks` (`chunk.rs:129`)**, retiring the adapter's Task 2b stopgap; add the `doc_chunk_spatial` sink. **Schema-shape decision resolved here, not deferred to a footnote:** persist `ChunkOut.bboxes` via a **sidecar relation keyed by `chunk_id`** (recommended) rather than adding a column to the 9-tuple `doc_chunks`, which is duplicated across ~15 query sites + the hardcoded `:create` in `provenance_engine.rs:43-95`. Add `schema.rs` `ensure_*` + `store::insert_*` for that relation. **Regenerate ts-rs bindings** — the workspace pins `ts-rs` and exports TS types, so any new `#[derive(TS)]` field on the spatial relation needs the TS export refreshed. Gated on importing the absent sidecar from god-agent. Adopting PR2 re-churns boundaries → another re-embed (§6). **Large.**
- **PR3 — Flip default + parity gate (M).** Import `plans/archon-ingestion-ports-spec.md §2` + `markdown_chunker.py` + golden corpus, stand up `scripts/chunk_parity_check.py` as CI differential (`.github/workflows/ci.yml`), resolve the cross-page `page_end` divergence **per the §4 acceptance criterion**, flip the `ChunkerKind` default to `TokenAware`, and run the §6.1 reindex/wipe + reprocess migration. **Medium.**

**Cross-cutting blockers to resolve before PR2/PR3 (assumptions, not resolved here):** canonical location of `plans/archon-ingestion-ports-spec.md §2`, `markdown_chunker.py`, and the golden corpus (god-agent repo / another branch); whether eager `retrieval::index_chunk` or the deferred `doc_index_queue` worker is the production embedding path (determines where re-embed cost lands); and final confirmation of the `doc_chunk_spatial` sidecar-relation key (recommended `chunk_id`).