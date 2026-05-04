# Provenance

Provenance records why an artifact exists and how it traces back to source
material. The Evidence Engine uses provenance for document chunks, answers,
game-theory reports, completion evidence, and later build/code artifacts.

> **TUI parity.** Every `archon prov <subcommand>` shell form has a `/prov <subcommand>` slash equivalent inside the TUI; same goes for `archon docs provenance` → `/docs provenance`. Both forms read the same persisted chain-hash and artifact-lineage rows. See [CLI and TUI Command Parity](cookbook/real-world-evidence-engine.md#cli-and-tui-command-parity).

## CLI

Current `archon prov --help` surface:

| Command | Purpose |
|---|---|
| `trace <artifact-id>` | Trace an artifact to source lineage |
| `export <artifact-id>` | Export lineage as W3C PROV JSON-LD |
| `verify <artifact-id>` | Verify the trace reaches source provenance |

Document-specific shortcut:

```bash
archon docs provenance <chunk-or-answer-id>
```

## What should be persisted

| Provenance data | Meaning |
|---|---|
| artifact id | stable id for a chunk, answer, report, section, or generated object |
| source id | source document, run, prompt, specialist, or input |
| chain hash | deterministic integrity hash over the chain |
| relation edges | links from derived artifacts to inputs |
| export shape | W3C PROV JSON-LD for external inspection |

## Full State Verification

```bash
archon docs ingest ./fixtures/policy-pack
archon docs inspect <document-id>
archon docs provenance <chunk-id>
archon prov trace <artifact-id>
archon prov export <artifact-id>
archon prov verify <artifact-id>
```

Do not treat an artifact id printed by a command as sufficient. The separate
trace/export/verify read must show a chain that reaches source material.
