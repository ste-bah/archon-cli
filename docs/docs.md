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
| `list` | List ingested documents | reads persisted document rows |
| `show <document-id>` | Show document metadata | one document |
| `status` | Show document status summary | row counts and processing status |
| `chunks <document-id>` | List chunks for a document | chunk source of truth |
| `inspect <document-id>` | Full document inspection | pages, chunks, OCR runs, provenance |
| `search <query>` | Search chunks | `--mode exact|semantic|hybrid`, `--debug` |
| `answer <query>` | Answer using document evidence | uses retrieved chunks |
| `provenance <chunk-or-answer-id>` | Show provenance chain | chunk or answer component |
| `index` | Embed and store vectors | `--all` re-indexes all chunks |
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

Supported inputs include Markdown, text, DOCX, PDF, PNG, JPEG, and TIFF. Native
text is chunked directly. PDFs and images run through the local OCR provider
when needed. Image files also record page/image hashes; optional VLM
descriptions and image embeddings are policy-gated and should either persist
real output or emit an explicit warning.

After ingest, inspect the persisted source of truth:

```bash
archon docs status
archon docs list
archon docs inspect <document-id>
archon docs chunks <document-id>
archon docs provenance <chunk-or-answer-id>
```

Then index and search:

```bash
archon docs index --all
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
| `hybrid` | Combines exact and semantic scores using policy weights |

Hybrid is the default:

```bash
archon docs search "quoted policy string" --mode exact
archon docs search "similar meaning to the policy" --mode semantic
archon docs search "policy marketplace incentives" --mode hybrid --debug
```

`--debug` prints retrieval internals such as query embedding norm, raw scores,
rerank scores when present, and citation/provenance chains.

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
| `/docs model-status` | Show embedding backend/vector state |

The read-side slash commands inspect the same Cozo source of truth as
`archon docs ...`; they are not canned TUI labels.

## Multimodal policy

Image OCR is local-provider first. Optional VLM descriptions are controlled by
policy and disabled by default:

```toml
[policy.docs.vlm]
enabled = false
mode = "disabled"
allow_cloud = false
require_user_confirmation_for_cloud = true
```

If image embeddings require a multimodal local embedding model that is not
available, ingest should continue with an explicit warning rather than pretending
image embeddings were produced.

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
