# Document Intelligence

Document intelligence ingests local files and directories, extracts text/OCR
evidence, chunks content, embeds chunks, and supports exact, semantic, and
hybrid retrieval with provenance.

> **TUI parity.** Every `archon docs <subcommand>` shell form has a `/docs <subcommand>` slash equivalent inside the TUI. Both forms read and write the same persisted Cozo state. See [CLI and TUI Command Parity](cookbook/real-world-evidence-engine.md#cli-and-tui-command-parity). When in the TUI, prefer the slash form — it runs through the in-session command dispatcher without leaving your conversation context.

## CLI

Current `archon docs --help` surface:

| Command | Purpose | Notes |
|---|---|---|
| `ingest <path>` | Ingest a file or directory | creates document, page/chunk/OCR/provenance state |
| `reprocess <target>` | Re-run OCR/VLM/image enrichment | target is a document ID, source path, or source path prefix; `--defer-index` skips the final semantic-index pass |
| `list` | List ingested documents | reads persisted document rows |
| `show <document-id>` | Show document metadata | one document |
| `status` | Show document status summary | row counts and processing status |
| `chunks <document-id>` | List chunks for a document | chunk source of truth |
| `inspect <document-id>` | Full document inspection | pages, chunks, OCR runs, provenance |
| `search <query>` | Search chunks | `--mode exact|semantic|hybrid`, `--debug` |
| `compile` | Summarize documents, extract concepts, refresh the index | `--kb`, `--model`; needs an LLM provider |
| `answer <query>` | Answer using document evidence | `--no-synthesis`, `--file`, `--kb`, `--limit`, `--mode`, `--model` |
| `export` | Dump the corpus to markdown | `--out <dir>` (default stdout), `--kb` |
| `provenance <chunk-or-answer-id>` | Show provenance chain | chunk or answer component |
| `index` | Embed and store vectors | `--all` re-indexes all chunks |
| `vector-status` | Show vector store state | legacy Cozo rows, RocksDB rows, HNSW snapshot |
| `vector-migrate` | Copy legacy vectors to RocksDB | resumable with `--after` |
| `vector-compact` | Build Rust-HNSW snapshot | uses RocksDB raw vectors |
| `model-status` | Report embedding backend | useful before ingest/index |

## Add Documents And Images

For a single file:

```bash
archon docs ingest ./policy-pack/marketplace-rules.pdf
archon docs ingest ./policy-pack/scanned-page.png
archon docs ingest ./policy-pack/research-notes.docx
```

For a directory:

```bash
archon docs ingest ./policy-pack
```

Supported inputs include Markdown, text, PDF, PNG, JPEG, and TIFF. Native text
is chunked directly. PDFs now use the unified three-pass path: `pdftotext`
extracts any text layer, `pdfimages` extracts embedded charts/figures/photos
for OCR plus optional VLM descriptions, and `pdftoppm` renders full pages only
for scanned/image-only fallback or when policy explicitly asks for page
rendering. Image files and PDF-derived images record page/image hashes; optional
VLM descriptions and image embeddings are policy-gated and should either
persist real output or emit an explicit warning.

After ingest, inspect the persisted source of truth:

```bash
archon docs status
archon docs list
archon docs inspect <document-id>
archon docs chunks <document-id>
archon docs provenance <chunk-or-answer-id>
```

For PDFs, `status` and `inspect` also show embedded images extracted, images
skipped by the icon/decorator filter, image OCR runs/failures, image VLM
descriptions/failures, and rendered page fallback counts.

## Compile: summaries, concepts, and an index

`archon docs compile` (REQ-KB-002) runs an LLM pass over every document ingested
since the last run and writes three kinds of derived document:

| Output | Stored as | Linked by |
|---|---|---|
| One summary per document | `archon-kb://summary/<document-id>` | `DerivedFrom` → the source document |
| Concept articles across the batch | `archon-kb://concept/<slug>` | `DerivedFrom` → each document cited |
| A single corpus index | `archon-kb://index` | none (rewritten each run) |

Cross-references between concept articles are `Cites` edges in
`doc_provenance_edges`.

These are **ordinary documents**, not a separate graph. That is the point: the
moment compile finishes, `docs search`, `kb search`, `kb recall` and
`kb process` all see the summaries and concepts without knowing compile exists.
A summary also inherits its source document's `--kb` memberships, so
`kb search --kb <name>` ranks the summaries drawn from that bucket.

```bash
archon kb ingest ./research-pack --kb trading-elliott-wave
archon docs compile --kb trading-elliott-wave
archon docs search "wave 3 invalidation"      # now also matches the summaries
```

Compilation is incremental. `compile_state.last_compiled_at` is the watermark;
a second run with nothing new ingested does no model work and reports zeros.
The pass never reads its own output back in, so summaries are never
re-summarized.

Progress is printed per document because NFR-PIPE-012 budgets five minutes for
twenty documents and every document costs one model round trip.

Compile requires a configured LLM provider — unlike `answer` there is no
meaningful extractive fallback, so it fails with an explicit message rather than
reporting a successful run of zero.

## Answer: extractive or synthesized

`archon docs answer` satisfies REQ-DOCS-013 (answer from retrieved context),
REQ-DOCS-014 (citations) and REQ-DOCS-015 (say so when the evidence is
insufficient). REQ-KB-003 specifies the same capability and adds two things,
which this command now does rather than a second verb:

- **LLM synthesis.** With a provider configured, retrieved chunks plus any
  compiled summaries and concept articles for those documents are synthesized
  into an answer. With no provider, or with `--no-synthesis`, the extractive
  path runs unchanged and prints cited evidence.
- **Filing.** `--file` stores the answer as a searchable document under
  `archon-kb://answer/<uuid>` with `DerivedFrom` edges to every chunk it cited,
  so `docs provenance <document-id>` walks from the answer to its evidence.

A filed answer is second-hand evidence, so it is scored 0.9× in later retrieval
(EC-PIPE-018): identical content ranks below the source it was synthesised from.
This is a multiplier, not an ordering guarantee — an answer that restates the
question and lists its citations can match more query terms than its source and
still come out on top.

### The synthesized answer streams

A synthesized answer prints token by token as the model produces it. Retrieval
is a rounding error next to the model round trip — measured on a live run, 9ms
of a 9.6s command — so the total is unchanged; what changes is that text starts
appearing in well under a second instead of after ten seconds of blank
terminal. Retrieval notes print above the answer, citations below it, and the
complete answer is what `--file` stores, exactly as before.

The command reports `Retrieval Nms, first token Nms, total Nms`. NFR-KB-003's
5-second budget is checked against the time to the first token, not the total:
with a streamed answer the first token is when the operator stops waiting, and
a warning that fires on every healthy run is one nobody reads.

The extractive path (no provider, or `--no-synthesis`) does not stream. It has
no model round trip to hide, so it prints its answer in one go and reports
`Retrieval Nms, synthesis Nms`.

```bash
archon docs answer "what is the retention window?"
archon docs answer "what is the retention window?" --file --kb trading-elliott-wave
archon docs answer "what is the retention window?" --no-synthesis --mode exact
```

An answer with no supporting evidence says so explicitly; it never fabricates
confidence.

## Export

`archon docs export` dumps the corpus to markdown with YAML frontmatter,
grouped into `raw/`, `compiled/`, `concepts/`, `answers/` and `index/` — the
same five groups the older `kb_nodes` export used, now resolved from
`doc_sources` by source path.

```bash
archon docs export                                  # single markdown stream to stdout
archon docs export --out ./kb-dump                  # one file per document, grouped
archon docs export --out ./kb-dump --kb trading-elliott-wave
```

Two frontmatter fields changed with the move off `kb_nodes`, rather than being
dropped: `domain_tag` became `kb` (the document's `doc_kb_memberships`), and
per-node `chunk_index` became `chunks` (the count, with the chunk text joined in
order). `derived_from` is new — the old export had no way to show provenance
edges.

## Repair Existing Evidence

If policy changes after ingest, or an older binary skipped an enrichment path,
use reprocess instead of ingesting the same file again. Ingest deduplicates by
content hash; reprocess preserves the existing document ID and KB memberships,
clears generated page/chunk/OCR/VLM rows for the document, then reruns the
policy-aware pipeline.

```bash
archon docs reprocess <document-id>
archon docs reprocess ./assets/research-paper/trading/trading-elliott-wave
archon docs reprocess <document-id> --defer-index
archon docs index
archon docs index --document <document-id> --batch-size 64
archon docs index --all --limit 500
archon docs index-status
archon docs index-retry-failed --limit 500
archon docs index-pause <job-id>
archon docs index-resume <job-id>
archon docs index-cancel <job-id>
archon docs index-daemon start --batch-size 64 --window-size 1024
archon docs index-daemon status
archon docs index-daemon stop
archon docs vector-status
archon docs vector-migrate --limit 1000 --batch-size 250
archon docs vector-compact
```

Inside the TUI:

```text
/docs reprocess <document-id>
/docs reprocess ./assets/research-paper/trading/trading-elliott-wave
/docs reprocess <document-id> --defer-index
/docs index
/docs index --document <document-id> --batch-size 64
/docs index-status
/docs index-retry-failed --limit 500
/docs index-pause <job-id>
/docs index-resume <job-id>
/docs index-cancel <job-id>
/docs index-daemon start
/docs index-daemon status
/docs vector-status
/docs vector-migrate --limit 1000 --batch-size 250
/docs vector-compact
```

Reprocess is intentionally in-place. If the source file content has changed,
Archon rejects the repair and asks you to ingest the changed file as a new
document so provenance does not silently mutate.

Use `--defer-index` for large repair batches. Each document is refreshed in
place, but the expensive global semantic indexing pass is left for one explicit
`docs index` run after the batch completes.

`docs index` first counts candidate chunks before loading an embedding model. If
there is no work, it exits without touching fastembed or a cloud endpoint.
`--document <id>` scopes a retry to one source, `--batch-size` controls provider
request size, and `--limit` lets you resume very large repairs in explicit
slices. Normal pending indexing is backed by durable `doc_index_queue` and
`doc_index_jobs` records: ingest enqueues pending chunks, index workers lease
bounded windows, failed rows remain visible, and `index-retry-failed` moves
failed rows back to pending. When `--limit` is omitted, Archon processes the
queue in bounded 1024-chunk windows instead of loading the full corpus at once.
Long indexing runs print flushed progress for queueing, leasing, bulk vector
storage, failures, and elapsed time from the terminal or TUI transcript.
Batch size adapts during the run: fast, healthy batches grow within bounded
limits, while provider errors shrink the next request. Duplicate chunk text
reuses an existing same-provider vector by content hash and records the chunk as
indexed without a second model call.
Embedding can run with an in-process worker pool while keeping one durable
writer. Set `ARCHON_DOCS_INDEX_EMBEDDING_WORKERS=2` and
`ARCHON_DOCS_INDEX_MAX_IN_FLIGHT_BATCHES=2` for a conservative first pass; keep
`ARCHON_DOCS_INDEX_WRITER_BATCH_SIZE=256` unless RocksDB write latency says
otherwise. Local fastembed only uses more than one worker when Archon starts
multiple model instances, controlled by `ARCHON_DOCS_FASTEMBED_INSTANCES`
(default: the requested worker count, capped at 4). Do not launch multiple
`docs index` processes for parallelism: the single foreground/daemon process
owns Cozo leases and serializes RocksDB/Cozo writes so queue rows are not
double-claimed.
The daemon form runs the same foreground index path in a separate Rust process,
with its pid in `.archon/run/docs-index-daemon.pid` and output in
`.archon/logs/docs-index-daemon.log`.

## Vector Store Migration

New semantic indexing writes raw embeddings to `.archon/doc-vector-store`
using RocksDB. Cozo remains the source of truth for documents, chunks, jobs,
provenance, and statuses; raw vectors move out of the hot Cozo write path so
large indexing runs avoid Cozo/HNSW lock contention. Rust-HNSW snapshots are
built separately from the RocksDB raw-vector store.

Existing vectors are not thrown away. Migrate legacy Cozo vector rows in safe
slices:

```bash
archon docs vector-status
archon docs vector-migrate --limit 1000 --batch-size 250
archon docs vector-migrate --after <last-chunk-id> --limit 1000
archon docs vector-compact
```

`vector-migrate` skips rows already present in RocksDB and prints a resume hint
with the last scanned chunk id. `vector-compact` builds the Rust-HNSW snapshot
for retrieval from the migrated or newly indexed vectors. Legacy Cozo vector
writes are opt-in only via `ARCHON_DOCS_LEGACY_COZO_VECTOR_WRITE=1`.

## Video Sources

Video evidence uses the `archon video` namespace and lands in the same
document/KB retrieval stack as ordinary files. Transcript, OCR, VLM, and summary
evidence are stored as `doc_chunks`, with `video_chunk_timeref` preserving
timecode provenance for `archon docs answer`.

```bash
archon video ingest ./lecture.mp4 --transcript ./lecture.vtt --frames none
archon video ingest "https://youtu.be/abc123" --frames hybrid --asr whisper-cpp --yes
archon video transcript <video-id> --format srt
archon video inspect <video-id>
```

See [Video Evidence Ingest](video.md) for transcript-only, ASR, YouTube,
frame-extraction, policy, local binary setup, and compliance workflows.

Then index and search:

```bash
archon docs index --all
archon docs index --document <video-document-id>
archon docs search "known phrase from the fixture" --mode exact --debug
archon docs search "similar meaning query" --mode semantic --debug
archon docs search "mixed phrase and concept" --mode hybrid --debug
```

## TUI Document Browser

Inside an interactive Archon session, use `/docs` for inspection:

```text
/docs open
/docs status
/docs list
/docs inspect <document-id>
/docs chunks <document-id>
/docs provenance <chunk-or-artifact-id>
/docs model-status
```

`/docs open` opens the TUI document/evidence browser and loads rows from the
same persisted document store used by `archon docs ...`. Ingestion itself is a
CLI operation today; the TUI is the read/inspection surface for checking what
was actually stored.

## Retrieval modes

| Mode | Behavior |
|---|---|
| `exact` | Uses full-text/exact content matching where available |
| `semantic` | Uses embedding/HNSW vector similarity |
| `hybrid` | Runs safe exact/FTS first, then semantic only when lexical evidence is not already strong enough |

Hybrid is the default:

```bash
archon docs search "quoted policy string" --mode exact
archon docs search "similar meaning to the policy" --mode semantic
archon docs search "policy marketplace incentives" --mode hybrid --debug
```

`--debug` prints retrieval internals such as query embedding norm, raw scores,
rerank scores when present, and citation/provenance chains.

Hybrid retrieval is staged for large corpora. User questions are normalized
before Cozo FTS so punctuation such as `?`, commas, and parentheses cannot
make the lexical stage fail. If the exact stage returns enough high-confidence
evidence, including a specific definition-style hit such as "what do you know
about the Hybrid System", Archon returns those cited chunks immediately instead
of loading the semantic vector path. Definition-style queries also get a narrow
relaxed exact retry before semantic fallback, so minor misses such as `systm`
can still hit strong `system` evidence. If exact evidence is weak, sparse, or
unavailable, hybrid falls through to embedding/HNSW retrieval and fuses the
scores as before.

For diagnostics, set `ARCHON_DOCS_HYBRID_ALWAYS_SEMANTIC=1` to force hybrid
search to run semantic retrieval even when exact evidence is already strong.

## Slash commands

Interactive sessions expose the document evidence browser through `/docs`:

| Slash form | Intent |
|---|---|
| `/docs open` | Open the TUI document/evidence browser |
| `/docs list` | List ingested documents |
| `/docs status` | Show persisted document/chunk/page counts |
| `/docs show <document-id>` | Inspect a document |
| `/docs inspect <document-id>` | Inspect a document |
| `/docs chunks <document-id>` | List persisted chunks |
| `/docs provenance <chunk-or-artifact-id>` | Show incoming/outgoing provenance edges |
| `/docs vector-status` | Show legacy Cozo, RocksDB, and Rust-HNSW vector state |
| `/docs vector-migrate` | Copy existing Cozo vectors into RocksDB without re-embedding |
| `/docs vector-compact` | Build a Rust-HNSW snapshot from RocksDB vectors |
| `/docs model-status` | Show embedding backend/vector state |

The read-side slash commands inspect the same Cozo source of truth as
`archon docs ...`; they are not canned TUI labels.

`/docs compile`, `/docs answer` and `/docs export` run through the CLI mirror
like `/docs ingest` does, so their progress and output stream into the session.
Because the mirror runs without a TTY, pass every flag on the command line.

## Multimodal policy

Image OCR is local-provider first. Optional VLM descriptions are controlled by
policy and disabled by default:

```toml
[policy.docs.vlm]
enabled = false
mode = "disabled"
provider = "disabled"
allow_cloud = false
require_user_confirmation_for_cloud = true

[policy.docs.vlm.ollama]
endpoint = "http://localhost:11434"
model = "gemma4:e4b"
timeout_secs = 120

[policy.docs.vlm.gemini]
api_key_env = "GOOGLE_API_KEY"
model = "gemini-3-flash-preview"
endpoint_base = "https://generativelanguage.googleapis.com/v1beta"
rpm_limit = 12

[policy.docs.vlm.anthropic]
model = "claude-sonnet-4-6"

[policy.docs.vlm.openai_compat]
endpoint = "http://localhost:1234/v1"
model = "google/gemma-3-12b-it"
api_key_env = "OPENAI_API_KEY"
timeout_secs = 120
max_tokens = 8192
temperature = 0.2

[policy.docs.pdf]
extract_embedded_images = true
min_image_dimension = 200
min_image_bytes = 4096
vlm_per_page_image = true
render_text_pdf_pages = false
image_enrichment_workers = 1
```

When enabled, VLM descriptions are stored in `doc_image_descriptions`, chunked into normal `doc_chunks`, and indexed by the existing text embedding backend. If an image embedding model is unavailable, ingest still succeeds; visual search works through the description chunks.

## Full State Verification

For ingestion:

```bash
archon docs ingest ./fixtures/policy-pack
archon docs status
archon docs list
archon docs inspect <document-id>
```

For retrieval:

```bash
archon docs search "known fixture phrase" --mode exact --debug
archon docs search "known synonym" --mode semantic --debug
archon docs search "mixed fixture query" --mode hybrid --debug
```

Expected physical evidence is persisted document rows, chunk rows, OCR rows for
images/PDF pages that require OCR, embedding/index rows after `index`, and
provenance edges linking chunks back to source documents.
