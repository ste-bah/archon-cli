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
| `list` | List all nodes | `--kb` |
| `search <query>` | Search nodes | `--limit`, `--mode exact|semantic|hybrid`, `--kb` |
| `process` | Extract structured intelligence from doc chunks | `--claims`, `--entities`, `--relations`, `--contradictions`, `--kb` |
| `claims` | List extracted claims | none |
| `entities` | List extracted entities | none |
| `relations` | List inferred relations | none |
| `contradictions` | List detected contradictions | none |
| `stats` | Show KB statistics | none |

## Source of truth

The expected persisted relations are claims, entities, relations, source-quality
records, and contradictions. `archon kb process` should write those rows from
real document chunks, and the list/search commands should read them back.

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
archon kb stats
```

Edge cases should include empty document stores, duplicate chunks, invalid
source paths, contradictory fixture claims, and searches with no matches.
