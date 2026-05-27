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
archon kb process --kb trading-elliott-wave --claims --entities --relations --contradictions
archon kb search --kb trading-elliott-wave "wave 3 invalidation" --mode hybrid
```

Inside the TUI, use the same slash forms:

```text
/kb ingest ./research-pack --kb trading-elliott-wave
/video ingest "https://youtu.be/abc123" --kb trading-elliott-wave --frames hybrid --asr whisper-cpp --yes
/kb process --kb trading-elliott-wave --claims --entities --relations --contradictions
/kb search --kb trading-elliott-wave "wave 3 invalidation" --mode hybrid
```

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
