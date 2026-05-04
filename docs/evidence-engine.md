# Archon Evidence Engine

The Evidence Engine turns Archon from a chat-first assistant into an inspectable
reasoning system. It stores documents, claims, provenance, game-theory runs,
completion-integrity checks, learning signals, meaning datasets, and
constellation centroids in queryable local state instead of leaving them as
transient model output.

## Combined architecture

| Layer | Crate or module | Source of truth | Main commands |
|---|---|---|---|
| Document intelligence | `archon-docs` | document, page, chunk, OCR, embedding, retrieval and provenance Cozo rows | `archon docs ...` |
| Knowledge extraction | `archon-knowledge` | claims, entities, relations, source quality, contradictions | `archon kb ...` |
| Provenance | `archon-provenance` plus document provenance | chain hashes and artifact lineage rows | `archon prov ...`, `archon docs provenance` |
| Game-theory pipeline | `archon-pipeline::gametheory` | `gt_runs`, fingerprints, routing, specialist outputs, sections, reports, checkpoints | `archon gametheory ...`, `/gametheory ...`, `GameTheory*` tools |
| Completion integrity | `archon-completion` | completion claims, evidence, gate results, incidents, run contexts, trust scores | `archon completion ...` |
| Governed learning | `archon-learning` and `src/command/behaviour.rs` | learning events, proposals, manifests, decisions | `archon behaviour ...`, `/learning-status` |
| Meaning compiler | `archon-meaning` | labeled samples, contrastive pairs, triplets | `archon meaning ...` |
| Constellation | `archon-constellation` | versioned centroid profiles | `archon constellation ...` |
| Policy | `archon-policy` | layered TOML policy files | read by feature gates |

## End-to-end flow

```bash
archon docs ingest ./policy-pack
archon docs index --all
archon kb process --claims --entities --contradictions
archon gametheory run "Assess the incentive structure of this plugin marketplace design" --budget 20 --max-concurrent 4
archon meaning build --from gametheory-runs
archon constellation build --target strategic-workflow
```

Every stage should leave physical evidence behind. Use the inspection commands
instead of trusting return values:

```bash
archon docs status
archon kb stats
archon gametheory list-runs
archon completion trust
archon meaning samples
archon constellation list
```

## Full State Verification pattern

For any feature or manual smoke test, define the trigger, the process, and the
stored outcome before calling it done.

| Step | What to do | Example |
|---|---|---|
| Trigger | Run the command or tool that should create state | `archon completion verify run-1 --agent verifier --model sonnet` |
| Process | Let the crate execute the real path | claim extraction, gate checks, incident recording, trust recompute |
| Outcome | Read the independent source of truth | `archon completion trust --agent verifier --model sonnet` |

Happy path and edge cases should include at least empty input, missing provider,
duplicate input, invalid format, interruption/resume, and contradictory content
where the feature accepts those shapes.

## Data locations

Most Evidence Engine commands use Cozo-backed SQLite files under the normal
Archon data directory. Some handlers also accept environment overrides:

| Area | Default or override |
|---|---|
| Docs | app data directory, `archon/docs.db`; no dedicated docs DB env override |
| KB | `ARCHON_KB_DB_PATH`, otherwise app data directory |
| Meaning | `ARCHON_MEANING_DB_PATH`, then `ARCHON_KB_DB_PATH`, otherwise app data directory |
| Constellation | `ARCHON_CONSTELLATION_DB_PATH`, then `ARCHON_MEANING_DB_PATH`, then `ARCHON_KB_DB_PATH`, otherwise app data directory |
| Completion integrity | `${XDG_DATA_HOME:-~/.local/share}/archon/archon-data.db` |
| Game-theory runs | same application data store used by the gametheory handler |
| Policy | `/etc/archon/policy.toml`, `~/.archon/policy.toml`, `<workspace>/.archon/policy.toml` |

## Project Initialisation

For a new project, run:

```bash
sh scripts/archon-init.sh --target /path/to/project --archon-cli-repo /path/to/archon-cli
```

The initialiser creates the Evidence Engine project tree:

| Path | Purpose |
|---|---|
| `.archon/policy.toml` | Safe local-first defaults for docs VLM, retrieval weights, learning gates, and game-theory Tier 11 |
| `.archon/specs/` | Routing/spec files such as `gametheory.yaml` |
| `.archon/docs/inbox/` | Optional drop zone for PDFs, DOCX, Markdown, text, and image files before `archon docs ingest` |
| `.archon/evidence/` | Workspace-local evidence artifacts and manual verification transcripts |
| `.archon/agents/` | Project agent definitions, including copied game-theory agents when available |
| `prds/` and `tasks/` | PRD-driven work artifacts |

Runtime databases still live in the application data directory by default; the
workspace tree holds policy, specs, input files, agents, and verification
artifacts.

See the focused guides for command details:

- [Document intelligence](docs.md)
- [Knowledge base](knowledge.md)
- [Game theory](gametheory.md)
- [Completion integrity](completion-integrity.md)
- [Governed learning](governed-learning.md)
- [Policy](policy.md)
- [Provenance](provenance.md)

## Derived learning commands

The meaning and constellation commands expose the derived-data side of governed
learning:

| Command | Purpose |
|---|---|
| `archon meaning build --from learning-events` | Build meaning samples from learning events |
| `archon meaning build --from gametheory-runs` | Build meaning samples from game-theory runs |
| `archon meaning samples` | List labeled samples |
| `archon meaning contrastive` | List contrastive pairs |
| `archon meaning triplets` | List triplets |
| `archon meaning export --kind samples|triplets` | Export JSONL datasets |
| `archon constellation build --target project|research-domain|strategic-workflow` | Build centroid profiles |
| `archon constellation score --target <target> --text <text>` | Score text or a file |
| `archon constellation drift --target <target> --text <text>` | Detect drift against a centroid |
| `archon constellation list` | List persisted centroids |
