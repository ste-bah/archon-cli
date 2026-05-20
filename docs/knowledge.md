# Knowledge Base

The knowledge base extracts structured claims, entities, relations, source
quality, and contradictions from document chunks. It is the bridge between raw
document intelligence and higher-level reasoning pipelines.

> **TUI parity.** Every `archon kb <subcommand>` shell form has a `/kb <subcommand>` slash equivalent inside the TUI. Both forms read and write the same persisted Cozo state. See [CLI and TUI Command Parity](cookbook/real-world-evidence-engine.md#cli-and-tui-command-parity). When inside an interactive session, prefer the slash form.

## CLI

Current `archon kb --help` surface:

| Command | Purpose | Important flags |
|---|---|---|
| `ingest <source>` | Ingest a file, URL, or directory into the KB | `--domain` |
| `list` | List all nodes | none |
| `search <query>` | Search nodes | `--limit`, `--mode exact|semantic|hybrid` |
| `process` | Extract structured intelligence from doc chunks | `--claims`, `--entities`, `--relations`, `--contradictions` |
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
supported document media: plain text, Markdown, PDF, PNG, JPEG, and TIFF. The
URL remains the stored source path while fetched bytes are passed through the
same hashing, duplicate detection, OCR/PDF/image/VLM policy gates, chunking,
and provenance rows. Additional text-like web formats such as HTML, JSON, XML,
YAML, and TOML are stored through the text-source path.

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
