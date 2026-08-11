# Knowledge Base

The knowledge base extracts structured claims, entities, relations, source
quality, and contradictions from document chunks. It is the bridge between raw
document intelligence and higher-level reasoning pipelines.

> **TUI parity.** Every `archon kb <subcommand>` shell form has a `/kb <subcommand>` slash equivalent inside the TUI. Both forms read and write the same persisted Cozo state. See [CLI and TUI Command Parity](cookbook/real-world-evidence-engine.md#cli-and-tui-command-parity). When inside an interactive session, prefer the slash form.

## CLI

Current `archon kb --help` surface:

| Command | Purpose | Important flags |
|---|---|---|
| `ingest <source>` | Ingest a file, URL, or directory into the KB | `--kb`, `--domain` alias |
| `list` | List all chunks | `--kb` |
| `search <query>` | Search chunks | `--limit`, `--mode exact|semantic|hybrid`, `--kb` |
| `recall <query>` | Merged recall across memory, docs, knowledge and the code index | `--sources`, `--limit`, `--code-index`, `--mode`, `--kb` |
| `process` | Extract structured intelligence from doc chunks | `--claims`, `--entities`, `--relations`, `--contradictions`, `--kb` |
| `reprocess` | Re-run OCR/VLM/image enrichment for a bucket | `--kb`, `--defer-index` |
| `claims` | List extracted claims | none |
| `entities` | List extracted entities | none |
| `relations` | List inferred relations | none |
| `contradictions` | List detected contradictions | none |
| `stats` | Show KB statistics | none |

## LLM synthesis lives under `docs`

Three capabilities operate on the same corpus but are LLM passes over documents
rather than structured extraction, so they sit in the `docs` namespace. They are
documented in full in [Document Intelligence](docs.md):

| Command | Purpose |
|---|---|
| `archon docs compile` | Summarize each document, extract concept articles, cross-reference them, refresh a corpus index (REQ-KB-002) |
| `archon docs answer` | Answer a question from retrieved evidence, with LLM synthesis and optional filing (REQ-KB-003 / REQ-DOCS-013–015) |
| `archon docs export` | Dump the corpus to markdown, grouped by kind |

All three read `doc_chunks` and write ordinary documents, so anything they
produce is immediately visible to `kb search`, `kb recall` and `kb process`.
A summary or filed answer inherits the `--kb` membership of what it came from,
which keeps bucket-scoped search honest:

```bash
archon kb ingest ./research-pack --kb trading-elliott-wave
archon docs compile --kb trading-elliott-wave
archon kb search --kb trading-elliott-wave "wave 3 invalidation"   # matches summaries too
archon kb process --kb trading-elliott-wave --claims --entities    # extracts from them too
```

## Source of truth

The expected persisted relations are claims, entities, relations, source-quality
records, and contradictions. `archon kb process` should write those rows from
real document chunks, and the list/search commands should read them back.

Compilation output is not a separate graph. `docs compile` writes `doc_sources`
and `doc_chunks` rows under an `archon-kb://` source path
(`archon-kb://summary/<document-id>`, `archon-kb://concept/<slug>`,
`archon-kb://index`), linked to their sources by `doc_provenance_edges`. Filed
answers use `archon-kb://answer/<uuid>`. The compile pass keeps one relation of
its own — `compile_state`, holding the `last_compiled_at` watermark.

> **Note on `kb_nodes`.** `archon-pipeline::kb::ingest` writes a parallel
> `kb_nodes` / `kb_edges` / `kb_content_hashes` / `kb_embeddings` graph. No CLI
> command populates or reads it. It is retained pending a separate removal
> decision and is not a supported second corpus — do not build against it.

URL ingest uses the same governed document pipeline as local ingest for
supported document media: plain text, Markdown, HTML, JSON, XML, YAML, TOML,
PDF, PNG, JPEG, and TIFF. The URL remains the stored source path while fetched
bytes are passed through the same hashing, duplicate detection, OCR/PDF/image/VLM
policy gates, chunking, indexing, and provenance rows.

## Named KB Buckets

Use `--kb <name>` to attach ingested sources to a durable KB bucket. The bucket
is a grouping over existing evidence documents, so it works for PDFs, images,
Markdown, text, URLs, and video evidence without duplicating chunks.

```bash
archon kb ingest ./research-pack --kb trading-elliott-wave
archon video ingest "https://youtu.be/abc123" --kb trading-elliott-wave --frames hybrid --asr whisper-cpp --yes
archon kb reprocess --kb trading-elliott-wave
archon kb reprocess --kb trading-elliott-wave --defer-index
archon kb process --kb trading-elliott-wave --claims --entities --relations --contradictions
archon kb search --kb trading-elliott-wave "wave 3 invalidation" --mode hybrid
```

Inside the TUI, use the same slash forms:

```text
/kb ingest ./research-pack --kb trading-elliott-wave
/video ingest "https://youtu.be/abc123" --kb trading-elliott-wave --frames hybrid --asr whisper-cpp --yes
/kb reprocess --kb trading-elliott-wave
/kb reprocess --kb trading-elliott-wave --defer-index
/kb process --kb trading-elliott-wave --claims --entities --relations --contradictions
/kb search --kb trading-elliott-wave "wave 3 invalidation" --mode hybrid
```

## Listing Buckets

`--kb <name>` is a filter, so every command above assumes you already know the
name. `kb kbs` is the one that tells you, listing every bucket the store holds
with its document count:

```bash
archon kb kbs
```

```text
/kb kbs
```

The name it prints first on each line is the exact string `--kb` matches. It is
not always the directory slug the web workbench shows under `.archon/kb/`: a
bucket created from the browser as "Trading research" is stored as
`Trading research` and shown as a `trading-research` directory, and `--kb`
wants the former.

The web workbench Ingest page lists the same set on its `kbs` tab, regardless of
whether a bucket was created there, from the CLI, or from the TUI. Each row
names its origin (`db`, `dir`, or `both`) so a bucket that exists on only one
side is visible rather than hidden, and a listing that could not read the store
says so instead of rendering as an empty tab.

`kb reprocess` repairs every document currently attached to the bucket. It keeps
the membership rows and document IDs, but refreshes generated OCR, PDF-image,
VLM-description, chunk, embedding, and provenance rows using the current project
policy.

For large KB repairs, use `--defer-index` and run `archon docs index` once after
the repair. That avoids repeating the global pending-vector sweep after every
document.

## Removing Documents From A Bucket

`kb reprocess` repairs a document in place. To take one out of the corpus
entirely, delete the underlying evidence document:

```bash
archon docs delete <document-id>
archon docs delete ./research-pack/superseded          # path prefix
archon docs delete ./research-pack/superseded --yes    # required for >1 match
```

```text
/docs delete <document-id>
/docs delete ./research-pack/superseded --yes
```

`--yes` is only required when the target is a path prefix that matches more than
one document; a single unambiguous match deletes without it. The command prints
each deleted document ID with its chunk count.

Delete removes the document row, its pages, chunks, embeddings and vector-index
entries, provenance/citation edges, pending index-queue jobs, and its
`--kb` bucket membership. The bucket itself survives — it is just a grouping, so
removing a member shrinks it rather than deleting it.

Two consequences worth knowing:

**Re-ingest is unblocked.** Ingest deduplicates on content hash, and that
registration lives on the document row itself. While the row exists, re-ingesting
the same bytes is skipped as a duplicate. Deleting the document releases the
hash, so the same content can be ingested again as a new document. This is the
supported recovery path when an ingest is interrupted partway and leaves a
document that is registered but incompletely processed.

**Extracted knowledge is not retracted.** Claims, entities, relations, and
contradictions produced by `kb process` are separate rows that reference the
document and chunk IDs. Deleting a document does not remove them, so
`archon kb claims` can still list claims whose source document is gone. Re-run
`kb process` after a round of deletions if downstream reasoning depends on the
extracted set matching the current corpus.

## Full State Verification

```bash
archon docs ingest ./fixtures/policy-pack
archon kb process --claims --entities --relations --contradictions
archon kb claims
archon kb entities
archon kb relations
archon kb contradictions
archon kb kbs
archon kb stats
archon docs compile
archon docs answer "what does the policy require?"
archon docs export --out ./kb-dump
```

Edge cases should include empty document stores, duplicate chunks, invalid
source paths, contradictory fixture claims, and searches with no matches.
