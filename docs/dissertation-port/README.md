# Archon Dissertation Port — Corpus Ingestion & Prose-Style Analytics

This document set describes the additions this fork
(`ProfessorVR/archon-cli`, branch `feat/marker-server-parallel-ingest`) makes on
top of the upstream `ste-bah/archon-cli` `main`. The delta is **91 files,
+13,670 / −152 lines**, and adds **three new workspace crates** plus two
end-user capabilities:

| Capability | New crates | CLI surface |
|---|---|---|
| **Corpus ingestion** — PDFs → Marker (surya) block trees with real bounding boxes → token-aware chunks → OCR/VLM enrichment → embedded vector index → tamper-evident integrity seal | `archon-accel` (device/VRAM placement), `archon-ingest-ext` (chunking + Marker parser); `archon-docs` heavily extended | `archon docs ingest / verify-integrity / reprocess / delete / index / search / verify-quote` |
| **Prose-style analytics** — Lanham prose-style fingerprinting that turns a writing sample into an enforceable Archon *output-style* | `archon-lanham` (pure-Rust analyzer + renderer) | `archon style train` |

Companion docs:
- **[platform-agnostic-design.md](platform-agnostic-design.md)** — how the same
  pipeline runs on Apple MPS, NVIDIA CUDA (incl. WSL), and CPU with byte-identical output.
- **[dependencies-and-setup.md](dependencies-and-setup.md)** — full dependency
  list and a clean getting-started runbook.

Everything is validated on real hardware: a 24 GB Apple-Silicon Mac (MPS), an
RTX 5070 8 GB laptop (CUDA), and an RTX 5090 32 GB workstation. The dissertation
corpus flip landed **124 documents, 123 with real Marker bboxes, integrity
124/124 clean**.

---

## 1. Corpus ingestion

### 1.1 What the original archon did

On upstream `main`, `crates/archon-docs/src/ingest_pdf.rs` is ~137 lines with **no**
reference to Marker, bounding boxes, block chunking, or tokens. PDFs were run
through `chunk_with_page_anchors`: flat `pdftotext` output split on page
offsets, with no per-block coordinates. There was no device/VRAM awareness and
no post-ingest integrity check. `archon-accel` and `archon-ingest-ext` did not
exist.

### 1.2 The pipeline now

Entry points `ingest_file_with_policy` / `ingest_directory_with_policy`
(`archon-docs`); PDFs run through `run_pdf_ingest_pipeline` (`ingest_pdf.rs`):

```
PDF
 └─ extract text + page_offsets + embedded images + page renders   (pdf::extract_pdf_unified)
 └─ per-page PageArtifact records                                   (store::insert_page)
 └─ Marker block tree  (surya neural layout + OCR, real per-block bboxes)
        via MarkerSource  →  archon_ingest_ext::marker::parse_marker_str
 └─ token-aware chunking  (800 / 1200 / 1400-token budgets)         (block_chunking + ingest_ext::chunk)
        each chunk carries page_start/page_end + per-page PageBoxes  → COORD_MARKER (or COORD_NONE on a Marker miss)
 └─ provenance edges  (chunk→page, artifact→document)               (provenance::build_doc_lineage_edges)
 └─ integrity seal  (per-chunk commit_hash + chunks_root)           (provenance_chunks::persist_chunk_integrity)
 └─ per-image enrichment  (image OCR + optional VLM figure descriptions, scanned-book aware)
 └─ embedding + vector index  (eager if a provider is configured; else `archon docs index`)
```

### 1.3 What is genuinely new

- **Real per-block bounding boxes** from Marker's surya neural layout/OCR,
  replacing flat `pdftotext` chunks — enables page+bbox citation and the
  `verify-quote` highlight path.
- **Token-aware, structure-aware chunking** (`archon-ingest-ext/chunk.rs`):
  800/1200/1400-token budgets, section-header boundary flushing, undersized
  pairwise merge, per-page super-boxes, and first-class `[TABLE]` chunks that
  preserve grid structure instead of flattening cells to prose.
- **Device-adaptive placement** (`archon-accel`): chooses the Marker device from
  **free VRAM, not card size**, models Marker's page-scaled VRAM footprint,
  page-range-chunks big documents to fit a small card, and carries a per-document
  GPU→CPU OOM fallback. See [platform-agnostic-design.md](platform-agnostic-design.md).
- **Persistent Marker throughput**: a FastAPI server
  (`scripts/archon_marker_server.py`, `MarkerSource::Http`) loads the ~6 GB surya
  models **once** instead of the per-document reload the subprocess sidecar pays.
  It requires `--pdf-root` and recursively freezes canonical regular PDFs into an
  immutable startup catalogue. Requests contain only a deterministic SHA-256 ID of
  the canonical UTF-8 pathname, not a path field or PDF bytes; nested PDFs and
  duplicate basenames remain distinct. Archon and server must resolve the same
  canonical pathname (same host or identically mounted filesystem). Unknown IDs,
  malformed requests, invalid page ranges, and conversion failures return fixed
  safe errors. Restart after corpus mutations: the catalogue chooses the
  startup-approved pathname and does not detect local replacement at that path.
  It binds to loopback by default; it has no authentication, and an explicit
  `--allow-non-loopback` accepts exposure without changing catalogue construction.
- **Integrity sealing**: a `chunks_root` tamper-evidence hash on *all* ingest,
  `docs verify-integrity` to prove no post-ingest tampering, an end-of-run
  `COORD_MARKER` vs `COORD_NONE` tally, and a **strict-fail** rule on the HTTP
  transport (a Marker error is a hard failure, never a silent bbox-less
  degradation).
- **VLM figure descriptions**: optional natural-language captions for embedded
  images and Marker figure regions, gated by an active scanned-book detector
  (a full-page scan is a page reproduction, not a figure), with `--jobs auto`
  VRAM-adaptive enrichment concurrency.
- **Hang-hardening**: per-marker-convert, per-OCR-subprocess, whole-document
  render, and per-image wall-clock timeouts (all with `kill_on_drop`), plus
  signal-kill→CPU retry — so no wedged external call can hang an ingest.

### 1.4 The `archon docs` CLI

| Subcommand | Purpose |
|---|---|
| `docs ingest <path> [--jobs auto\|N] [--yes]` | Ingest a file or directory. `--jobs auto` derives image-enrichment concurrency from free VRAM; `--yes` skips the pre-ingest confirmation. |
| `docs verify-integrity [--doc <id>] [--json]` | Recompute + compare the `chunks_root`; classifies each doc pass / fail / no-record. |
| `docs reprocess <target> [--defer-index]` | Re-run OCR/VLM/image enrichment for an existing document. |
| `docs delete <target> [--yes]` | Permanently remove a document and all its evidence, including the content-hash registration so the same content can be re-ingested. `--yes` is required when `<target>` is a prefix matching several documents. |
| `docs index [--all] [--document <id>] [--batch-size N]` | Embed + store chunk vectors (deferred-indexing path); plus `index-status`, `index-daemon`, … |
| `docs search <query> [--mode exact\|semantic\|hybrid]` | Chunk retrieval (default hybrid); `--debug` prints distances + citation chains. |
| `docs verify-quote <quote> [--doc <id>] [--json]` | Locate a quote's source document, page(s), and bbox(es). |

Example: `archon docs ingest ~/corpus --jobs auto --yes && archon docs verify-integrity`

---

## 2. Prose-style analytics (`archon-lanham`)

### 2.1 What the original archon did

Upstream archon had **no** prose fingerprinting. Its only style machinery is
coarse and hand-configured: `research/style.rs` injects US/UK spelling +
citation-style + formality boilerplate into research prompts, and
`research/final_stage/style_applier.rs::apply_style` is an explicit pass-through
stub (`TODO(REQ-RESEARCH-007)`) that returns the paper unchanged.

### 2.2 What archon-lanham adds

A pure-Rust, fully-offline (no Node, no LLM, no network) port of a Lanham
prose-style analyzer. It measures a corpus's style along six axes and renders
the result as an enforceable Archon *output-style*:

| Axis | Emitted metrics |
|---|---|
| Noun/Verb | `noun_verb_ratio`, `nominalization_density`, `prepositional_phrase_density`, `be_verb_ratio` |
| Parataxis/Hypotaxis | `parataxis_hypotaxis_ratio`, coordinating/subordinating conjunction density |
| Periodic/Running | `periodic_running_ratio`, `pre_main_verb_clause_count` |
| Voice | `voice_score`, `dynamic_range` |
| Register | `latinate_germanic_ratio`, `register_markedness_score` |
| Opacity | `opacity_score`, `self_consciousness_score` (blended with tacit rhetorical figures) |

Plus tacit-persuasion figure detectors (alliteration, polyptoton, chiasmus,
antithesis, anaphora, isocolon, climax) and human-readable labels banded against
academic-genre thresholds. `full_analysis(text)` returns the complete
`LanhamMetrics`. Output is golden-tested byte-for-byte against the original
TypeScript/`.mjs` reference.

### 2.3 The `archon style` CLI + how it's used

```
archon style train [FILES...] [--name NAME] [--genre GENRE] [--out PATH] [--stdout]
```

`train` measures the sample(s) (or stdin) and renders an Archon output-style
`.md` to `~/.archon/output-styles/<name>.md`. At session start, Archon loads
output-styles from that directory; selecting one (`--output-style <name>` or the
`output_style` config key) appends its body — binding stylistic constraints like
*"Write in a high, formal academic register…"*, *"Connection is predominantly
PARATACTIC…"* — to the system prompt, so the model drafts in the trained voice.

```bash
archon style train samples/*.md --name dalton-philosophical --genre academic
archon --output-style dalton-philosophical "...your prompt..."
```

The analyzer is pure Rust (`regex`, `once_cell`, `serde`) — no external tools.

---

## 3. Improvements vs original archon — at a glance

| Area | Upstream `main` | This fork |
|---|---|---|
| PDF → chunks | flat `pdftotext`, page-offset splits, no coordinates | Marker surya block tree, real per-block bboxes |
| Chunking | char/page anchored | token-aware (800/1200/1400), structure-aware, `[TABLE]` chunks |
| Device/VRAM | none | free-VRAM placement, page-scaled footprint, page-range chunking, GPU→CPU ladder, Apple unified budget (`archon-accel`, new) |
| Marker throughput | n/a | per-doc sidecar **or** warm persistent HTTP server |
| Integrity | none | `chunks_root` seal + `verify-integrity` + COORD tally + strict-fail |
| Figures | none | VLM figure descriptions, scanned-book aware, `--jobs auto` |
| Robustness | n/a | per-op timeouts + signal-kill→CPU retry (no ingest hangs) |
| Prose style | coarse spelling/citation boilerplate + a stub | `archon-lanham` quantitative Lanham fingerprint → output-style (new) |

---

## 4. Known limitations / experimental (honest inventory)

**Ingestion**
- The `ConsumerKind::{Whisper, FrameVlm}` variants in `placement.rs` are an
  intentional, near-zero-cost seam — **not pending work**. They exist so the
  accel type surface stays honest for a future revisit of video, but the
  multi-consumer *arbiter* they'd feed is deliberately unbuilt: GPU consumers on
  the media paths run **sequentially** (Marker exits and frees its VRAM before
  the VLM loads; video ASR → frame-VLM would follow the same shape), so
  single-consumer free-VRAM placement already covers the real cases and there is
  no co-residency to budget. Marker is the only live GPU consumer; the
  whisper/frame-VLM footprints are placeholder estimates. If constrained-hardware
  video ingest ever needs it, the useful move is to route those consumers through
  the existing single-consumer placement — not to build the arbiter.
- Embedding is CPU-only this round (GPU embedding is opt-in / future).
- `figure_region_vlm` (figure-crop VLM) and non-default scan-detector modes are opt-in.
- `archon_marker_core.py` should be re-confirmed against the installed Marker
  version's converter/renderer import paths and `bbox` vs `polygon` box shape.

**Style analytics**
- POS/clause features are deferred ("L2"): the `tag_pos` seam returns empty, so
  "that"-subordination, participial clauses, and the register F-score are
  approximated or stubbed. The en-pos tagger + clause parser are future ("L3").
- Genre thresholds are academic-only; `--genre` mainly changes renderer phrasing.
- Rust-trained profiles are leaner than the full god-agent reference (empty
  hedges/transitions → the `## CLAIMS` / `## TRANSITIONS` output-style sections
  are omitted unless a rich profile is supplied).
- The multi-agent research pipeline's `style_applier` is still a stub — the
  Lanham output-style feeds the interactive session prompt, not the research
  pipeline. Wiring them together is future work.
