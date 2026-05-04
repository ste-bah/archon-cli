# Document Intelligence

Document intelligence ingests local files and directories, extracts text/OCR
evidence, chunks content, embeds chunks, and supports exact, semantic, and
hybrid retrieval with provenance.

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
