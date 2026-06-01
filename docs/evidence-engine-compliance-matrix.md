# Evidence Engine Compliance Matrix

Generated: 2026-05-04

Scope audited:
- PRD: `/home/unixdude/Archon-projects/archon/prds/memory/PRD-ARCHON-EVIDENCE-ENGINE-001.md`
- TSPEC: `/home/unixdude/Archon-projects/archon/prds/memory/TSPEC-ARCHON-EVIDENCE-ENGINE-001.md`
- SOURCE-MAP: `/home/unixdude/Archon-projects/archon/prds/memory/SOURCE-MAP-ARCHON-EVIDENCE-ENGINE-001.md`
- Worktree: `/home/unixdude/Archon-projects/archon-cli-worktrees/evidence-engine-phase6`
- Branch: `evidence-engine/phase6-governed-learning`

Status legend:
- `DONE`: implementation has code, persistence/source-of-truth evidence, and focused tests or CLI wiring.
- `PARTIAL`: implementation exists, but final PRD-level behavior still needs live transcript proof or one sub-surface is not proven.
- `GAP`: no credible implementation evidence found.

Audit result: 54 `DONE`, 18 `PARTIAL`, 0 `GAP`.

This matrix is not a replacement for the final PRD §5 integration transcript. It is a code-audited compliance map showing what is implemented and what still needs full-state verification in the final smoke run.

## Completion Integrity

| ID | Status | Evidence | Notes |
| --- | --- | --- | --- |
| REQ-COMP-001 | PARTIAL | `crates/archon-completion/src/evidence_resolver.rs`, `src/command/completion.rs` | Command evidence is persisted and checked, but live final-answer enforcement in the interactive answer path still needs transcript proof. |
| REQ-COMP-002 | DONE | `crates/archon-completion/src/claim_extractor.rs` | Completion-sensitive claims are extracted and carried into verification. |
| REQ-COMP-003 | DONE | `crates/archon-completion/src/incident_recorder.rs`, `crates/archon-completion/src/store.rs` | False-completion incidents persist to `false_completion_incidents`. |
| REQ-COMP-004 | PARTIAL | `crates/archon-completion/src/report_assembler.rs`, `src/command/completion.rs` | Report-state assembly exists; final prose path needs end-to-end proof. |
| REQ-COMP-005 | PARTIAL | `crates/archon-completion/src/verifier.rs`, `crates/archon-completion/src/evidence_resolver.rs` | Contradiction/gate blocking exists, but full contradiction-to-final-answer block needs live proof. |
| REQ-COMP-006 | DONE | `crates/archon-completion/src/incident_recorder.rs`, `crates/archon-learning/src/outcome_signal.rs` | User correction/false-completion rows now create canonical `learning_events`. |
| REQ-COMP-007 | DONE | `crates/archon-completion/src/trust.rs`, `src/command/completion.rs` | Trust scores compute from incident/completion history and persist to `agent_model_trust_scores`. |
| REQ-COMP-008 | PARTIAL | `src/command/completion.rs`, `src/command/evidence_view.rs`, `crates/archon-tui/src/events.rs` | CLI incident inspection exists; TUI evidence rows exist, but incident-specific TUI browsing needs transcript proof. |

## Document Intelligence

| ID | Status | Evidence | Notes |
| --- | --- | --- | --- |
| REQ-DOCS-001 | DONE | `crates/archon-docs/src/ingest.rs` | Ingest handles files/directories and supported document/image media. |
| REQ-DOCS-002 | PARTIAL | `crates/archon-docs/src/ocr/local.rs`, `crates/archon-docs/src/ingest.rs` | PDF/image OCR paths exist; native-vs-image-vs-mixed page detection needs live fixture proof. |
| REQ-DOCS-003 | PARTIAL | `crates/archon-docs/src/ocr/provider.rs`, `crates/archon-docs/src/ingest.rs` | OCR is page-aware; selective page/region OCR is not fully proven by matrix evidence. |
| REQ-DOCS-004 | DONE | `crates/archon-docs/src/ingest.rs`, `crates/archon-docs/src/store.rs` | Duplicate detection uses content hashes. |
| REQ-DOCS-005 | DONE | `crates/archon-docs/src/chunk.rs`, `crates/archon-docs/src/store.rs` | Page boundaries and offsets are persisted. |
| REQ-DOCS-006 | DONE | `crates/archon-docs/src/ingest.rs`, `crates/archon-docs/src/store.rs` | Chunk IDs and page ranges persist in `doc_chunks`. |
| REQ-DOCS-007 | DONE | `crates/archon-docs/src/embed.rs`, `crates/archon-docs/src/retrieval.rs` | Chunks embed through configured provider with explicit missing-provider errors. |
| REQ-DOCS-008 | DONE | `crates/archon-docs/src/vector_store.rs`, `crates/archon-docs/src/vector_migration.rs`, `crates/archon-docs/src/retrieval.rs` | RocksDB raw-vector storage, legacy Cozo vector migration, and Rust-HNSW snapshot search are wired. |
| REQ-DOCS-009 | DONE | `crates/archon-docs/src/schema.rs`, `crates/archon-docs/src/retrieval.rs`, `src/command/docs.rs` | Exact, semantic, and hybrid modes exist with FTS and weighted scoring. |
| REQ-DOCS-010 | DONE | `crates/archon-docs/src/answer.rs`, `src/command/docs.rs` | Answers persist citation provenance edges from answer to cited chunks. |
| REQ-DOCS-011 | PARTIAL | `src/command/docs.rs`, `crates/archon-docs/src/retrieval.rs` | Search debug shows norms/scores/provenance; answer-specific debug mode still needs final proof. |
| REQ-DOCS-012 | DONE | `src/command/evidence_view.rs`, `crates/archon-tui/src/app.rs` | `/docs open/view` loads persisted document rows into TUI state. |

## Multimodal

| ID | Status | Evidence | Notes |
| --- | --- | --- | --- |
| REQ-MM-001 | PARTIAL | `crates/archon-docs/src/ingest.rs` | Standalone image ingest is implemented; embedded image extraction from PDF/DOCX needs fixture proof. |
| REQ-MM-002 | DONE | `crates/archon-docs/src/ocr/local.rs` | Local PDF rendering/OCR path exists. |
| REQ-MM-003 | DONE | `crates/archon-docs/src/ingest.rs`, `crates/archon-docs/src/store.rs` | Image/page hashes and page provenance are persisted. |
| REQ-MM-004 | DONE | `crates/archon-docs/src/ingest.rs`, `crates/archon-docs/src/embed.rs` | Image embeddings use provider capability when available and warn explicitly otherwise. |
| REQ-MM-005 | DONE | `crates/archon-docs/src/vlm.rs`, `crates/archon-policy/src/decision.rs` | VLM descriptions are policy-gated and disabled by default. |
| REQ-MM-006 | PARTIAL | `crates/archon-docs/src/ingest.rs` | VLM/OCR content is linked into chunks, but figure/table-to-caption association needs stronger fixture proof. |

## Provenance

| ID | Status | Evidence | Notes |
| --- | --- | --- | --- |
| REQ-PROV-001 | DONE | `crates/archon-docs/src/ingest.rs`, `crates/archon-pipeline/src/gametheory/schema.rs`, `crates/archon-provenance/src/store.rs` | Derived artifacts persist parent/child edges. |
| REQ-PROV-002 | DONE | `crates/archon-docs/src/store.rs`, `crates/archon-docs/src/ingest.rs` | Source documents retain content hashes. |
| REQ-PROV-003 | DONE | `crates/archon-docs/src/answer.rs`, `src/command/docs.rs` | Answer outputs carry answer IDs and citation edges. |
| REQ-PROV-004 | DONE | `crates/archon-provenance/src/chain.rs`, `crates/archon-provenance/src/export_w3c.rs` | Chain hashing and W3C JSON-LD export are implemented. |
| REQ-PROV-005 | PARTIAL | `crates/archon-provenance/src/traverse.rs`, `src/command/prov.rs`, `crates/archon-docs/src/answer.rs` | Answer traversal is now backed by edges; full gametheory report chain needs final transcript proof. |

## Game Theory

| ID | Status | Evidence | Notes |
| --- | --- | --- | --- |
| REQ-GT-001 | DONE | `.archon/agents/gametheory/`, `crates/archon-pipeline/src/gametheory/registry.rs` | Registry and guard tests cover 84 game-theory agents and exclusions. |
| REQ-GT-002 | DONE | `crates/archon-pipeline/src/gametheory/facade.rs` | Tier 1 mandatory agents execute through the real facade. |
| REQ-GT-003 | DONE | `crates/archon-pipeline/src/gametheory/fingerprint.rs` | 9-axis fingerprint model is implemented and persisted. |
| REQ-GT-004 | DONE | `.archon/specs/gametheory.yaml`, `crates/archon-pipeline/src/gametheory/routing.rs` | YAML routing covers all registry agents with conditions/dependencies. |
| REQ-GT-005 | DONE | `crates/archon-pipeline/src/gametheory/facade.rs`, `crates/archon-pipeline/src/gametheory/spec.rs` | Enabled specialists run in dependency-respecting parallel waves. |
| REQ-GT-006 | DONE | `crates/archon-pipeline/src/gametheory/routing.rs`, `crates/archon-pipeline/src/gametheory/schema.rs` | Routing decisions persist as auditable artifacts. |
| REQ-GT-007 | DONE | `crates/archon-pipeline/src/gametheory/final_stage/` | Section writers and final strategic report assembly are implemented. |
| REQ-GT-008 | DONE | `crates/archon-pipeline/src/gametheory/schema.rs`, `src/command/gametheory.rs` | Cost fields, budget cap, and partial-report behavior are wired. |
| REQ-GT-009 | DONE | `src/command/gametheory.rs`, `crates/archon-policy/src/decision.rs` | Tier 11 is gated by flag and policy. |
| REQ-GT-010 | DONE | `crates/archon-pipeline/src/gametheory/schema.rs`, `src/command/gametheory.rs` | Run artifacts persist in Cozo with gametheory surfaces. |

## Governed Learning

| ID | Status | Evidence | Notes |
| --- | --- | --- | --- |
| REQ-LEARN-001 | PARTIAL | `crates/archon-learning/src/store.rs`, `crates/archon-learning/src/outcome_signal.rs`, `crates/archon-completion/src/incident_recorder.rs` | Canonical learning events exist; every retrieval/routing/output source needs final integrated proof. |
| REQ-LEARN-002 | PARTIAL | `crates/archon-knowledge/src/source_quality.rs`, `crates/archon-learning/src/apply.rs` | Source-quality updates exist; automatic update coverage across all outcomes needs proof. |
| REQ-LEARN-003 | PARTIAL | `crates/archon-pipeline/src/gametheory/routing.rs`, `crates/archon-learning/src/store.rs` | Routing confidence learning is represented but not fully proven end-to-end. |
| REQ-LEARN-004 | DONE | `crates/archon-meaning/src/` | Samples, contrastive pairs, triplets, and exports are implemented. |
| REQ-LEARN-005 | DONE | `crates/archon-constellation/src/` | Centroids, scoring, and drift detection are implemented. |
| REQ-LEARN-006 | DONE | `crates/archon-learning/src/manifest.rs`, `crates/archon-learning/src/apply.rs` | Versioned manifests and apply/rollback workflow exist. |
| REQ-LEARN-007 | DONE | `crates/archon-learning/src/policy.rs`, `crates/archon-policy/src/decision.rs` | High-impact learning changes require approval through policy gates. |

## Non-Functional Requirements

| ID | Status | Evidence | Notes |
| --- | --- | --- | --- |
| NFR-001 | DONE | `crates/archon-policy/src/models.rs`, `crates/archon-docs/src/embed.rs` | Local-first defaults and provider isolation are represented. |
| NFR-002 | PARTIAL | `crates/archon-docs/src/retrieval.rs`, `crates/archon-pipeline/src/gametheory/facade.rs` | Non-blocking/latency paths have tests, but final foreground latency transcript still needed. |
| NFR-003 | DONE | `crates/archon-pipeline/src/gametheory/facade.rs`, `crates/archon-docs/src/ingest.rs` | Long operations are async/bounded with warnings rather than UI-blocking assumptions. |
| NFR-004 | DONE | `crates/archon-pipeline/src/gametheory/schema.rs`, `src/command/gametheory.rs` | Resume/checkpoint command and `gt_run_checkpoints` relation exist. |
| NFR-005 | DONE | `crates/archon-provenance/src/chain.rs`, `crates/archon-docs/src/answer.rs` | Provenance persistence and chain hashing are implemented. |
| NFR-006 | DONE | `src/command/gametheory.rs`, `crates/archon-pipeline/src/gametheory/schema.rs` | Budget/cost controls are implemented. |
| NFR-007 | PARTIAL | `src/command/evidence_view.rs`, `src/command/docs.rs`, `src/command/gametheory.rs`, `src/command/completion.rs` | Inspection surfaces exist; complete operator UX needs final transcript. |
| NFR-008 | DONE | `crates/archon-llm/src/providers/`, `crates/archon-docs/src/embed.rs` | Provider abstraction keeps LLM/embedding providers isolated. |
| NFR-009 | DONE | `crates/archon-learning/src/policy.rs`, `crates/archon-policy/src/decision.rs` | Governed learning defaults to approval for risky changes. |
| NFR-010 | DONE | `crates/archon-policy/src/decision.rs`, `crates/archon-policy/src/models.rs` | Network and MCP-like risky access are policy-first/default-deny. |

## Acceptance Criteria

| ID | Status | Evidence | Notes |
| --- | --- | --- | --- |
| AC-001 | PARTIAL | `src/command/docs.rs`, `src/command/kb.rs`, `src/command/gametheory.rs`, `src/command/meaning.rs`, `src/command/constellation.rs` | All command surfaces exist; full PRD §5 transcript still must be captured. |
| AC-002 | DONE | `src/command/completion.rs`, `crates/archon-completion/src/verifier.rs` | Completion verification blocks unverified claims through evidence/gate state. |
| AC-003 | DONE | `crates/archon-docs/src/retrieval.rs` | Hybrid retrieval is implemented and tested against exact/semantic fixtures. |
| AC-004 | DONE | `crates/archon-docs/src/ingest.rs`, `crates/archon-docs/src/answer.rs` | Citations and provenance edges persist. |
| AC-005 | DONE | `.archon/specs/gametheory.yaml`, `crates/archon-pipeline/src/gametheory/registry.rs` | All game-theory specialists are represented in routing. |
| AC-006 | DONE | `crates/archon-pipeline/src/gametheory/facade.rs`, `crates/archon-pipeline/src/gametheory/schema.rs` | Game-theory run artifacts persist. |
| AC-007 | DONE | `crates/archon-pipeline/src/gametheory/final_stage/` | Final report is assembled from section writers. |
| AC-008 | DONE | `crates/archon-learning/src/`, `src/command/behaviour.rs` | Learning proposals are governed and inspectable. |
| AC-009 | DONE | `crates/archon-meaning/src/` | Meaning compiler builds training/evaluation artifacts. |
| AC-010 | DONE | `crates/archon-constellation/src/` | Constellation build/score/drift surfaces exist. |
| AC-011 | DONE | `crates/archon-provenance/src/`, `src/command/prov.rs` | Provenance trace/export/verify CLI exists. |
| AC-012 | PARTIAL | `crates/archon-policy/src/decision.rs` | Policy gates exist; every risky external surface needs final policy transcript. |
| AC-013 | DONE | `src/command/evidence_view.rs`, `crates/archon-tui/src/app.rs` | TUI view rows are loaded from persisted sources. |
| AC-014 | PARTIAL | `docs/evidence-engine.md`, `docs/gametheory.md`, `docs/docs.md`, `docs/completion-integrity.md`, `docs/governed-learning.md`, `docs/policy.md`, `docs/provenance.md` | Docs exist and were de-staled; full help-vs-doc parity should be checked before release. |

## Verification Evidence

Focused commands run safely during this audit/fix stream:

```bash
CARGO_BUILD_JOBS=2 cargo test -j1 -p archon-completion evidence_resolver --no-fail-fast -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -j1 -p archon-completion incident_recorder --no-fail-fast -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -j1 -p archon-learning outcome_signal --no-fail-fast -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -j1 -p archon-tui open_view --no-fail-fast -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -j1 -p archon-cli-workspace evidence_view --no-fail-fast -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -j1 -p archon-cli-workspace gametheory_slash --no-fail-fast -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo test -j1 -p archon-docs answer --no-fail-fast -- --test-threads=1
CARGO_BUILD_JOBS=2 cargo check -j1 -p archon-completion
CARGO_BUILD_JOBS=2 cargo check -j1 -p archon-learning
CARGO_BUILD_JOBS=2 cargo check -j1 -p archon-docs
CARGO_BUILD_JOBS=2 cargo check -j1 -p archon-cli-workspace
```

Final release gate still required:

```bash
archon docs ingest ./policy-pack
archon kb process --claims --entities --contradictions
archon gametheory "Assess the incentive structure of this plugin marketplace design" --kb policy-pack
archon meaning build --from gametheory-runs
archon constellation build --target strategic-workflow
```

## Highest-Risk Remaining Proof Items

| Area | Status | Required proof |
| --- | --- | --- |
| Final answer integrity | PARTIAL | Capture a run where false completion evidence changes or blocks final completion output. |
| PDF/DOCX embedded image extraction | PARTIAL | Fixture with embedded image proves extraction, OCR, hashes, and caption/page linkage. |
| Full provenance traversal | PARTIAL | Trace `answer -> chunk -> page -> source` and `report -> section -> specialist -> fingerprint -> situation` from persisted rows. |
| Learning feedback loop | PARTIAL | Show retrieval/routing/output/user-feedback events all become learning events and alter downstream quality/routing state. |
| Help/docs parity | PARTIAL | Snapshot every Evidence Engine CLI/slash `--help` or help output and compare against docs. |
