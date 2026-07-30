# Ingestion Parity — Divergence Register

**Status: LOCKED by the user on 2026-06-30. These are EXPECTED, intentional divergences — NOT drift.**

Reviewers and CI MUST NOT flag any item in this register as a parity regression. The
god-agent Python ingestion (`markdown_chunker.py` + `run_ingest_phase2.py`) is the parity
source-of-truth for the *chunk-boundary algorithm and constants only*. Everything in the
table below is a deliberate Rust improvement that the user has decided to KEEP; the Rust
port (`crates/archon-ingest-ext` + `crates/archon-docs`) is **not** to be "fixed" back
toward the Python behavior for these items.

- **Python source-of-truth** (god-agent tree):
  - `scripts/ingest/markdown_chunker.py`
  - `scripts/ingest/run_ingest_phase2.py`
  - `scripts/ingest/layout_analyzer.py`
  - (god-agent repo: `/home/dalton/projects/claudeflow-testing`)
- **Rust port** (this repo):
  - `crates/archon-ingest-ext/src/chunk.rs`, `layout.rs`, `table.rs`
  - `crates/archon-docs/src/provenance_chunks.rs`

Every `file:line` anchor below was grep/sed-verified against the working tree on 2026-06-30.

---

## Expected divergences

| # | Divergence | Python behavior + anchor | Rust behavior + anchor | Decision | Parity impact |
|---|-----------|--------------------------|------------------------|----------|---------------|
| 1 | **Tables: flatten vs structured grid** | `Table` is a member of `_TEXT_BLOCK_TYPES` (`markdown_chunker.py:230`), so a Marker `Table` block is treated like prose: its `html` is run through `_extract_text_from_html` (`markdown_chunker.py:245-247`, `re.sub(r'<[^>]+>', '', html)`) during the tree walk (`:277`) — the grid is **flattened to tag-stripped prose**. | Builds a structured grid: `parse_table_html` → `Vec<Vec<String>>` (`table.rs:191`), gated by the `is_real_table` quality check (`table.rs:52`), then serialized CSV/Markdown/JSON and emitted as a `[TABLE] Page …` chunk via `table_chunk_text` (`table.rs:177`). | **Keep Rust structured.** Python flatten is a quality loss (cell structure destroyed). | Text of any chunk covering a table **region differs** (Rust = `[TABLE] …` + Markdown grid; Python = run-together cell prose). |
| 2 | **`chunks_root` / `commit_hash` provenance (Rust-only)** | **No equivalent.** `grep -n 'chunks_root\|commit_hash'` returns **0 hits** in both `markdown_chunker.py` and `run_ingest_phase2.py`. | Per-chunk Merkle-style integrity: `commit_hash` (`provenance_chunks.rs:35`) over `chunk_id ∥ raw_sha256 ∥ clean_sha256 ∥ spatial_hash ∥ cleaning_version`, rolled into a sorted `chunks_root` (`provenance_chunks.rs:48`) that becomes an `extract_text_spatial` provenance record. | **Keep as a Rust addition** (tamper-evidence on all ingest). | Rust emits provenance rows/fields Python has no counterpart for. Nothing to compare — this is additive, not a difference in shared output. |
| 3 | **Text cleaning: `clean_v1` vs `none`** | `clean_corpus_text` (`run_ingest_phase2.py:546`) runs a deterministic v1 pipeline; `CLEANING_VERSION = "clean_v1"` (`run_ingest_phase2.py:505`). Step 1 strips standalone locator lines (bare page numbers / Bekker refs) via `_LOCATOR_RE` (`run_ingest_phase2.py:515-523`, applied `:562-565`). Stored as `clean_sha256` ≠ `raw_sha256` (`run_ingest_phase2.py:1729-1734`), `cleaning_version` metadata `:1748`. | **No cleaning.** `CLEANING_VERSION = "none"` (`provenance_chunks.rs:24`); the stored content IS the extracted content, so `raw_sha256 == clean_sha256 == content_hash` (`provenance_chunks.rs:9-10`, `:69`). | **Keep `none`.** | `clean_sha256` / `raw_sha256` digests differ **by design**. Compare the **scheme** (which hash equals which), not the digest values. Cross-system digest equality is NOT expected. |
| 4 | **Locator capture: anchored (standalone) vs unanchored (also inline)** | `extract_locators_with_bbox` (`markdown_chunker.py:307`) scans block text with **unanchored** `re.finditer` (`:323`) over `_LOC_BEKKER_RE = \b\d{2,4}[ab]\d{0,3}\b` (`markdown_chunker.py:303`) and `_LOC_STEPH_RE = \b\d{2,3}[a-e]\d{0,2}\b` (`markdown_chunker.py:304`) → captures locators **embedded inside body prose** as well as running heads. | `extract_locators` pulls a block out as a locator **only when its ENTIRE trimmed text matches** (`layout.rs:50-57`), using the **anchored** `bekker_re = ^\s*\d{1,4}[ab]\d{0,3}\s*$` (`layout.rs:44`) → **standalone running-head locators only.** | **Keep Rust anchored.** | `total_locators` **differs** (Python captures strictly more — every inline Bekker/Stephanus hit in body prose; Rust captures only standalone running heads). |
| 5 | **`page_end` of a flushed chunk: last-page-contained vs next-chunk-first-page** | When a block triggers a flush, `current_page_end` has **already been advanced to the incoming block's page** (`markdown_chunker.py:404`, set before the flush check at `:414-417`), so the flushed chunk's `page_end` is the **first page of the *next* chunk**. | The flush happens **before** the triggering block is added (`chunk.rs:139-143`); `page_end` is only updated by `Accum::add` (`chunk.rs:96`) and read on flush (`chunk.rs:117`), so a chunk's `page_end` is the **last page it actually contains.** | **Keep Rust correction** (citation-accurate page spans). | A multi-block chunk immediately preceding a page break can show a `page_end` one greater in Python than in Rust. Expected; Rust is the citation-correct value. |
| 6 | **Bekker regex broadened (catches 4-digit Aristotle numbers, requires column letter)** | Two reference Bekker regexes, both **narrower** than the port: (a) the strip-and-discard `layout_analyzer.py:54` `_BEKKER_RE = ^\s*\d{2,3}[a-b]?\d{0,2}\s*$` — **caps at 3 leading digits** (so it **misses 4-digit** Bekker numbers like `1147a`) and makes the **column letter optional** (so letterless numbers are conflated with page numbers); (b) the bbox-locator `markdown_chunker.py:303` `_LOC_BEKKER_RE = \b\d{2,4}[ab]\d{0,3}\b`. | `bekker_re = ^\s*\d{1,4}[ab]\d{0,3}\s*$` (`layout.rs:44`): **1–4 leading digits** and a **REQUIRED** column letter `[ab]`. Matches real Aristotle citations (`184b15`, `1147a`, `1147a13`) and keeps Bekker distinct from page numbers (rationale in `layout.rs:8-13`). | **Keep Rust broadened regex.** | Set of strings recognized as Bekker locators differs. Note: the genuine *4-digit* gain (`1147a`) is relative to `layout_analyzer.py:54`'s `\d{2,3}` cap — `markdown_chunker.py:303`'s `\d{2,4}` already reaches 4 digits, so vs that regex the only difference is the lower bound (`{1,4}` vs `{2,4}`) plus anchoring. See anchor-verification note below. |

### Anchor-verification note on Divergence #6

The PR-A divergence brief paraphrased the Python side as "`\d{2,4}…`", which is the literal
`markdown_chunker.py:303` `_LOC_BEKKER_RE`. That regex already reaches **four** leading
digits, so the "catches 4-digit Aristotle numbers like `1147a`" benefit does **not** hold
against it. The regex that actually **caps at 3 digits and therefore misses `1147a`** is the
strip-and-discard `layout_analyzer.py:54` `_BEKKER_RE = \d{2,3}[a-b]?\d{0,2}` — which is also
exactly the regex the Rust module docstring (`layout.rs:8-13`) cites as the one it broadens.
The table above documents both Python regexes with their verified anchors so the "4-digit"
claim is attributed to the correct one.

---

## Must-match (byte-exact) — chunk-boundary constants

These constants define the shared chunk-boundary algorithm and **MUST stay byte-identical**
across both systems. A change to any of them on one side without the other is real drift and
**must** be flagged.

| Constant | Value | Python anchor | Rust anchor |
|----------|-------|---------------|-------------|
| `TARGET_MIN` (tokens) | `800` | `markdown_chunker.py:26` (`TARGET_MIN_TOKENS = 800`) | `chunk.rs:12` (`pub const TARGET_MIN: usize = 800`) |
| `TARGET_MAX` (tokens) | `1200` | `markdown_chunker.py:27` (`TARGET_MAX_TOKENS = 1200`) | `chunk.rs:13` (`pub const TARGET_MAX: usize = 1200`) |
| `HARD_MAX` (tokens) | `1400` | `markdown_chunker.py:28` (`HARD_MAX_TOKENS = 1400`) | `chunk.rs:14` (`pub const HARD_MAX: usize = 1400`) |
| `CHARS_PER_TOKEN` | `4` | `markdown_chunker.py:31` (`CHARS_PER_TOKEN = 4`) | `chunk.rs:15` (`const CHARS_PER_TOKEN: usize = 4`) |

Related must-match algorithmic invariants (already ported, called out so they are not
mistaken for divergences):

- **Token estimate = Unicode code points / 4**, not bytes (`chunk.rs:53-59` vs Python
  `len(text)//4` at `markdown_chunker.py:34-36`). Byte length would shift every flush
  boundary on Greek/German text.
- **Undersized-merge is strictly pairwise** (`i += 2`, no re-evaluation of the merged
  chunk): Rust `merge_undersized` (`chunk.rs:184-200`) mirrors Python
  `chunk_marker_json`'s merge loop (`markdown_chunker.py:437-478`).
- **Flush triggers**: adding a block would exceed `max`, OR a `SectionHeader` arrives once
  the current chunk is `>= min` (`chunk.rs:133-144` vs `markdown_chunker.py:414-417`).

---

## How to run the parity check

The harness runs the **Python reference** chunker (no torch / no GPU needed — it operates on
already-parsed Marker JSON) so its output can be diffed against the Rust `chunk_blocks` port.

From the repo root (`/home/dalton/projects/archon-cli`):

```bash
# Reference lives in the god-agent tree; override the path if it moved.
export CHUNKER_REF_DIR=/home/dalton/projects/claudeflow-testing/scripts/ingest

# (1) Built-in fixture smoke test — the default, no-argument invocation.
python3 scripts/chunk_parity_check.py

# (2) Real Marker JSON dump (PR-D corpus diffing) — additive mode.
python3 scripts/chunk_parity_check.py --marker-json path/to/marker_dump.json
```

A **clean run of (1)** prints the reference chunk list as indented JSON and exits `0`:

```json
[
  {
    "page_start": 1,
    "page_end": 1,
    "text_head": "aaaaaaaaaaaaaaaaaaaaaaaa",
    "text_len": 3600,
    "bbox_pages": [
      1
    ]
  },
  {
    "page_start": 1,
    "page_end": 2,
    "text_head": "Section Two\n\nshort tail ",
    "text_len": 28,
    "bbox_pages": [
      1,
      2
    ]
  }
]
```

A **clean run of (2)** prints a per-chunk reference table (`idx`, `page_start`, `page_end`,
`text_len`, `text_head`) for the real document so PR-D can diff it against the Rust port's
chunk boundaries on real corpus PDFs. When diffing, expect the constants (above) and the
boundary algorithm to match exactly; expect the six items in the divergence register to
differ — that difference is the *contract*, not a failure.
