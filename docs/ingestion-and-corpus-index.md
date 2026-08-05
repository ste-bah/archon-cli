# Document Ingestion & the Corpus Index

*How archon turns source PDFs into byte-addressable, quote-verifiable evidence —
and how to operate the pipeline. Written 2026-08-05, current as of the native
coordinate extraction + mandatory verification gates.*

---

## 1. What the system does

The pipeline has one governing goal: **citation precision with provenance**.
Every sentence of every ingested document should be addressable down to its
page and bounding box, and every claim/clause entry in the corpus index should
be machine-verifiable against the exact source text it cites. Nothing enters
silently; nothing degrades silently.

It is organized as three stages with verification built into each:

```
source PDF ──▶ [1. INGESTION] ──▶ [2. SEMANTIC INDEXING] ──▶ retrieval-ready
                    │
                    ▼                    [3. CORPUS-INDEX ENTRIES]
        chunks + sentences + bboxes ◀─── clauses / claims / edges
        + locators + provenance          (quote-gated at import)
```

### Stage 1 — Ingestion (one command, fully gated)

`archon docs ingest <file|dir>` runs, per document:

1. **Media detection + dedup.** Content-hashed; byte-identical duplicates are
   skipped and linked to the existing document.
2. **Scan classification.** A three-detector union (aspect, coverage, and
   their combination) decides scanned vs born-digital. The verdict drives both
   extraction routing and whether page-scan images skip wasteful enrichment.
3. **Coordinate extraction.** Three routes, best-first:
   - **Born-digital → native (`coord_space = "pdf-native"`).** The PDF's own
     glyph positions are read via `pdftotext -tsv` by
     `scripts/archon_pdf_native_sidecar.py` — no OCR, no GPU, ~2 s/document,
     and *mathematically exact* boxes rather than OCR approximations. The
     sidecar de-hyphenates line breaks, strips running heads and repeated
     watermarks (bare page numbers are kept so locator capture can store
     them), applies a conservative 2-column reorder (a no-op when poppler's
     reading order already agrees), and detects table grids, emitting
     Marker-compatible JSON parsed by the existing Marker parser.
   - **Scanned → Marker** (GPU OCR with real bboxes), when configured.
   - **Fallback → flat text** (`coord_space = "none"`): page-accurate, no
     boxes. Never silent — the coord space is recorded and summarized.
   Failure at a better route falls through to the next; a configured Marker
   URL retains its hard-fail guarantee (no silent bbox-less degradation).
4. **Chunking + spatial persistence.** Token-aware chunking with per-chunk
   spatial rows (`doc_chunk_spatial`), per-block boxes (`doc_chunk_blocks` —
   the sentence layer's bbox source), page-break maps, and standalone
   page-number/Bekker locator capture (`doc_locators`).
5. **Sentence layer, inline.** Byte-spanned, sha-verified sentences with
   per-sentence page and tight bbox. Built *before* status flips — a document
   cannot read `Ingested` without a sentence layer matching its text. This
   invariant now holds for `docs reprocess` as well (same gate order:
   sentence rebuild → admissibility → status).
6. **Admissibility gate.** Ligature-dropout markers, degenerate extraction
   (e.g. zero chunks for a text-bearing type), missing sentences, or a
   bbox-less Marker fallback **fail the document** (`Failed` status) rather
   than admitting it. A damaged source is a loud reject, not quiet debt.
7. **Integrity + provenance.** Per-chunk tamper-evident hashes fold into a
   `chunks_root`; lineage edges connect document → artifact → chunks → pages.

### Stage 2 — Semantic indexing (one command, self-auditing)

`archon docs index` drains a durable queue: embeds every pending chunk
(local fastembed by default), stores raw vectors (RocksDB) and maintains the
HNSW snapshot. Every run — including on an empty queue — ends with the
**completion audit**: chunks of `Ingested` documents that are not `indexed`
are counted, and a full pass leaving any behind is a **hard error**. A green
`docs index` is therefore a standing attestation that retrieval sees the
whole corpus.

Embedding is deliberately decoupled from ingest for bulk work (see §4).

### Stage 3 — Corpus-index entries (authored, quote-gated at the door)

The corpus index is the claim/clause-level layer: `corpus_sources`,
`corpus_clauses` (verbatim quotes with chunk/byte/page/bbox anchors),
`corpus_claims`, `corpus_edges`, `corpus_tensions`, `corpus_groups`. Entries
are authored by the index-entry creation process (human or agent) as JSONL
intermediates and enter the store through:

```
archon corpus-index import <kind> <file.jsonl>
```

Imports are schema-validated, batched as keyed upserts, and **quarantined per
record** on rejection — nothing is silently dropped; every rejected row lands
in a `.quarantine.jsonl` sidecar with its reason.

For `clauses`, the **quote-verification gate runs by default**: every anchored
row (one carrying an `archon:<doc-id>` text layer) must locate its quote in
that document — exact match, or fuzzy similarity ≥ 0.90 — or it is quarantined
instead of written. *Entries are born verified.* Unanchored rows (cited-only
sources with no archon document) skip the gate by design. Opt out only with
`--no-verify-quotes` (not recommended), audit without writing via `--dry-run`.

---

## 2. How-to: operating the pipeline

### Where source documents go

Place PDFs under the repo's `corpus/` directory, one folder per topical
category (the folder name is organizational, not semantic):

```
corpus/
  boredom/
  metaphysics/
  new_media/
  rhetorical_ontology/
  ...
```

Rules of thumb:
- **Only source documents** live in `corpus/` — no notes, no fixtures, no
  index data. (`corpus/` is gitignored; documents are data, not code.)
- Filename convention: `Author - Title_(Year).pdf` variants are fine; the
  filename becomes the human-readable handle in listings and reports.
- Born-digital PDFs need no preparation. Scanned PDFs are auto-detected.
- Damaged sources (broken ToUnicode maps, fontless print-to-PDF artifacts)
  will be **rejected by the admissibility gate** — that is the system working.
  Find a cleaner copy or schedule a Marker OCR pass.

### Ingest

```bash
# Single file or whole directory — from the REPO ROOT (paths and the store
# resolve relative to the working directory):
archon docs ingest -y ./corpus/new_media/

# Bulk ingest: defer embedding (see §4 for why), then index once:
ARCHON_DOCS_EMBEDDING_PROVIDER=disabled archon docs ingest -y ./corpus/
archon docs index
```

`-y` skips the interactive enrichment-classification confirmation. The end of
a directory ingest prints the **coordinate integrity summary**:

```
PDF coord: 0 doc(s) COORD_MARKER, 9 COORD_PDF_NATIVE (real bboxes), 0 COORD_NONE (text fallback)
```

Any `COORD_NONE` count is warned loudly — investigate before moving on.

### Inspect and spot-verify

```bash
archon docs status                      # corpus census: Ingested / Failed counts
archon docs list                        # every document with status + path
archon docs show <doc-id>               # one document in detail
archon docs chunks <doc-id>             # chunk inventory + embedding status
archon corpus-index probes              # per-document bbox/spatial/locator coverage
archon docs verify-quote "some exact sentence from a source" \
    # → document, page, bbox, exact/fuzzy verdict — the citation-precision payoff
```

### Reprocess (re-run extraction for existing documents)

```bash
archon docs reprocess <doc-id | path | path-prefix> --defer-index
archon docs index
```

Reprocess preserves document IDs, clears derived evidence, re-runs the full
pipeline, rebuilds the sentence layer, and applies the same admissibility
gate as fresh ingest. Note: regenerated chunk boundaries can strand
corpus-index clause anchors — re-anchor affected rows by quote afterwards
(their stored quotes + hashes make this mechanical).

### Create and import index entries

```bash
# 1. Author entries as JSONL (one record per line) per the corpus-v1 schema.
# 2. Validate the intermediate:
archon corpus-index validate clauses my-entries.jsonl
# 3. Rehearse the import — schema + quote gate, writes nothing:
archon corpus-index import clauses my-entries.jsonl --dry-run
# 4. Import (quote gate ON by default for clauses):
archon corpus-index import clauses my-entries.jsonl
# 5. Check the result + audit trail:
archon corpus-index status
```

Rejected rows are never lost: read `<file>.quarantine.jsonl` for per-row
reasons (`quote not found in doc-…`, `similarity 0.62 < 0.9`, schema errors),
fix, and re-import — the keyed upsert makes re-imports idempotent.

### Verify the whole corpus (batch-close / weekly)

After any batch operation (bulk ingest, reprocess campaign, migration, bulk
import), run the corpus-global verification scan — four layers, ~20 checks:
document-store integrity (status census, `chunks_root` hashes, sentence-layer
presence, coverage probes), vector layer (queue drained, every Ingested
chunk indexed), corpus-index referential integrity (every clause→chunk/doc,
claim→clause, edge→endpoint reference resolves), and sampled content truth
(random clause quotes re-verified against their pinned documents). In
deployments that carry the harness: `scripts/verify-corpus.sh` (the harness
embeds corpus-specific baselines, so it travels with the corpus operator).

---

## 3. The verification guarantees, summarized

| Boundary | Guarantee | Enforced by |
|---|---|---|
| Ingest / reprocess | `Ingested` ⇒ sentence layer matches text; degenerate extraction ⇒ `Failed` | inline sentence build + admissibility gate |
| Indexing | full pass ⇒ zero unindexed chunks of Ingested docs | completion audit (hard error) |
| Entry import | anchored clause ⇒ quote verified exact-or-≥0.90 | quote gate (default for clauses) + quarantine |
| Corpus (global) | referential integrity + sampled quote-truth | batch-close scan |

---

## 4. Forward-looking: known architectural improvements

1. **Indexing writer/HNSW serialization (the big one).** Eager per-chunk
   indexing rebuilds and dumps the full HNSW snapshot per insertion — ~8
   min/chunk at ~27k vectors — which is why bulk flows defer embedding and
   batch it. The batch indexer amortizes this to one rebuild per window, but
   the serialized vector-store write + snapshot tail still dominates its wall
   clock. The fix is to debounce snapshot maintenance (rebuild once per run,
   or mark-dirty + compact on demand) and parallelize the writer; once landed,
   ingest and indexing can safely collapse into a single command
   (`docs ingest --index`), making the pipeline literally one-shot.
2. **FTS query sanitization.** Hyphenated and punctuation-heavy quotes
   (`fractional-delay`, `( Mem .1.449b29`) currently fail FTS parsing and fall
   to a capped scan; sanitizing the query builder recovers a class of
   verify-quote misses (observed as ~4% of re-anchor attempts on a real
   corpus).
3. **Re-anchor-on-reprocess.** Reprocessing regenerates chunk boundaries;
   clause anchors re-verify mechanically from stored quotes, but today that is
   an operator step. Folding an automatic re-anchor pass for affected rows
   into reprocess would close the loop.
4. **Marker-recovery lane for defective sources.** Fontless print-to-PDF
   artifacts and ligature-damaged text layers are correctly rejected today;
   a queued Marker-OCR retry lane would convert them instead of parking them.
5. **PyMuPDF diacritic normalization.** The rich extraction path (font sizes,
   `find_tables`) stays opt-in until its combining-diacritic rendering
   ("pathē" → "path¯e") is normalized; with that fixed it could replace
   heuristic header/table detection for mixed-format corpora.
6. **Completion-audit surfacing.** The audit currently reports at the end of
   `docs index`; surfacing the same counter in `docs status` would make the
   attestation visible in the everyday census view.
