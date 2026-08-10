pub(crate) async fn handle_draft_command(
    pack_path: PathBuf,
    workdir: PathBuf,
    model: Option<String>,
    gate_config: Option<PathBuf>,
    config: &ArchonConfig,
) -> Result<()> {
    let pack_dir = pack_path.parent().unwrap_or_else(|| Path::new("."));

    let pack: Pack = serde_json::from_str(
        &std::fs::read_to_string(&pack_path)
            .with_context(|| format!("read pack {}", pack_path.display()))?,
    )
    .with_context(|| "parse pack")?;

    let bank_path = pack_dir.join(&pack.p4b_bank_path);
    let bank: QuoteBank = serde_json::from_str(
        &std::fs::read_to_string(&bank_path)
            .with_context(|| format!("read bank {}", bank_path.display()))?,
    )
    .with_context(|| "parse bank")?;

    let gc_path =
        gate_config.unwrap_or_else(|| pack_dir.join(&pack.p2_style_target.gate_config_path));
    let cfg: GateConfig = serde_json::from_str(
        &std::fs::read_to_string(&gc_path)
            .with_context(|| format!("read gate config {}", gc_path.display()))?,
    )
    .with_context(|| "parse gate config")?;

    // --model → configured Anthropic Opus → built-in default (claude-opus-4-8).
    let model = fable::resolve_model(
        model.as_deref(),
        Some(config.models.anthropic.opus.as_str()),
    );
    eprintln!("archon draft: model={model} work={}", workdir.display());

    // Values needed for the post-run provenance import (the run moves pack/model/workdir).
    let section_id = pack.meta.section_id.clone();
    let model_for_import = model.clone();
    let chain_path = workdir.join("provenance.jsonl");
    // The provenance store path is resolved once, here at the command boundary, and
    // threaded into the import below as an explicit argument. Resolving it several
    // frames down left tests no way to point the import at a temporary store except
    // by mutating the process environment, which races the rest of the suite (#166).
    // The `ARCHON_PROV_DB_PATH` / `ARCHON_KB_DB_PATH` overrides behave as before.
    let prov_db_path = crate::command::store_paths::evidence_db_path(
        crate::command::store_paths::PROV_DB_ENV_KEYS,
    );
    // Quote (id, verbatim text) pairs for post-run corpus-source linkage — captured before the
    // bank moves into the blocking task.
    let quote_texts: Vec<(String, String)> = bank
        .iter()
        .map(|(id, q)| (id.clone(), q.text.clone()))
        .collect();

    let outcome = tokio::task::spawn_blocking(
        move || -> std::result::Result<orchestrator::Outcome, String> {
            let fclient = FableClient::from_env().map_err(|e| e.to_string())?;
            let call = |p: &str, mt: u32| fclient.call(&model, p, mt);
            orchestrator::run(&call, &model, &pack, &bank, &cfg, &workdir)
                .map_err(|e| e.to_string())
        },
    )
    .await
    .map_err(|e| anyhow!("draft task panicked: {e}"))?
    .map_err(|e| anyhow!("draft run failed: {e}"))?;

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "status": outcome.status,
            "cycles": outcome.cycles,
            "surface_to_user_in_skeleton": outcome.surface_to_user_in_skeleton,
            "chain_verified": outcome.chain_verified,
            "surfaced_defects": outcome.surfaced_defects,
            "final_words": outcome.final_draft.split_whitespace().count(),
        }))?
    );

    // Promote the JSONL chain into the shared CozoDB provenance store (best-effort: a store
    // failure must not fail a completed draft). Once imported, `archon prov trace|export|
    // verify <artifact-id>` work on the FCDP artifacts.
    match import_provenance_to_store(
        &prov_db_path,
        &chain_path,
        &section_id,
        &model_for_import,
        &quote_texts,
        &outcome.final_draft,
    ) {
        Ok(Some(summary)) => {
            eprintln!(
                "archon draft: imported {} provenance records; trace with `archon prov trace {}`",
                summary.records, summary.final_artifact
            );
            for s in &summary.cited {
                eprintln!(
                    "archon draft: cited source {} (p.{}\u{2013}{}, {}{:.2}) \u{2190} {} \u{2014} {}",
                    s.document_id,
                    s.page_start,
                    s.page_end,
                    if s.exact { "exact " } else { "~" },
                    s.similarity,
                    s.quote_ids.join(","),
                    s.source_path
                );
            }
            if summary.cited.is_empty() {
                eprintln!(
                    "archon draft: no corpus sources linked (quotes did not resolve in this store); `archon prov verify` reports reaches_source=false"
                );
            } else {
                eprintln!(
                    "archon draft: {} corpus source(s) linked; `archon prov verify {}` now reaches source",
                    summary.cited.len(),
                    summary.final_artifact
                );
            }
        }
        Ok(None) => {}
        Err(e) => eprintln!("archon draft: provenance store import skipped ({e:#})"),
    }

    Ok(())
}

/// Map an FCDP stage to a coarse artifact type for the provenance store.
fn artifact_type_for(stage: &str) -> &'static str {
    match stage {
        "d1-plan" => "movement-plan",
        "d15-skeleton" => "skeleton",
        "d2-assembled" => "draft",
        "revision" => "draft-revision",
        s if s.starts_with("d2-m") => "movement-draft",
        s if s.starts_with("gauntlet") || s == "regauntlet" || s == "g1-gate" => "gate-report",
        _ => "artifact",
    }
}

/// Below this quote→corpus similarity a source is not linked (avoids a wrong-source citation
/// edge from an incidental FTS collision). Verbatim, verified quotes resolve at ~1.0.
const CITE_LINK_FLOOR: f64 = 0.85;

/// A corpus source the finished draft quotes, resolved from the FCDP quote bank.
struct CitedSource {
    document_id: String,
    source_path: String,
    page_start: u32,
    page_end: u32,
    similarity: f64,
    exact: bool,
    quote_ids: Vec<String>,
}

/// Outcome of importing an FCDP chain into the shared store.
struct ImportSummary {
    records: usize,
    final_artifact: String,
    /// Ingested source documents the draft was linked to (empty when nothing resolved).
    cited: Vec<CitedSource>,
}

/// Strip LaTeX/markdown markup and surrounding quotation punctuation from a bank quote so it
/// matches the ingested source verbatim (the corpus is plain pdftotext / Marker text).
fn clean_quote(text: &str) -> String {
    archon_draft::strip_markup(text)
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`' || c.is_whitespace())
        .to_string()
}

/// Import an FCDP JSONL provenance chain into the shared CozoDB store as archon-provenance
/// records + `DerivedFrom` edges, then link the assembled draft to the ingested corpus its
/// surviving quotes come from (a `Cites` edge to each cited source document and to the exact
/// chunk(s) each quote occupies). Returns the import summary, or `None` if the chain is empty.
/// Store records use archon-provenance's
/// chain-hash rule (so `archon prov verify` works); the JSONL keeps FCDP's own hash for its
/// self-contained verify. Corpus linkage is content-verified and best-effort: only quotes that
/// survive verbatim into the final draft AND resolve into the corpus are linked; an unresolved
/// quote (corpus absent, index cold) simply produces no edge.
///
/// `db_path` is the provenance store to write into, resolved by the caller at the command
/// boundary rather than read from the environment here.
fn import_provenance_to_store(
    db_path: &Path,
    chain_path: &Path,
    section_id: &str,
    model: &str,
    quotes: &[(String, String)],
    final_draft: &str,
) -> Result<Option<ImportSummary>> {
    use archon_provenance::record::{ProvenanceEdge, ProvenanceEdgeType, ProvenanceRecord};

    let chain = archon_draft::provenance::read_chain(chain_path)
        .with_context(|| format!("read chain {}", chain_path.display()))?;
    if chain.is_empty() {
        return Ok(None);
    }

    let db = crate::command::store_paths::open_sqlite_db(db_path, "provenance")?;
    archon_provenance::store::ensure_schema(&db)?;

    let now = chrono::Utc::now().to_rfc3339();
    let mut prev_artifact: Option<String> = None;
    let mut prev_record_id: Option<String> = None;
    let mut prev_output_hash: Option<String> = None;
    let mut prev_chain_hash: Option<String> = None;
    let mut final_artifact = String::new();
    // The node the quotes enter at (D2 substitution); citation edges anchor here and are reached
    // from every downstream revision + the final artifact.
    let mut assembled_artifact: Option<String> = None;

    for (i, r) in chain.iter().enumerate() {
        // Stage-scoped, index-prefixed id: unique per record, order-revealing, portable.
        let artifact_id = format!("{section_id}#{i:02}-{}", r.stage);
        if r.stage == "d2-assembled" {
            assembled_artifact = Some(artifact_id.clone());
        }
        let parent_hashes: Vec<String> = prev_chain_hash.iter().cloned().collect();
        let input_hashes: Vec<String> = prev_output_hash.iter().cloned().collect();
        let parent_record_ids: Vec<String> = prev_record_id.iter().cloned().collect();

        let ch = archon_provenance::chain::chain_hash(
            &parent_hashes,
            &r.stage,
            &input_hashes,
            &r.content_sha256,
            Some("fcdp"),
            Some(model),
            &r.detail,
        );

        let record = ProvenanceRecord {
            record_id: r.record_id.clone(),
            artifact_id: artifact_id.clone(),
            artifact_type: artifact_type_for(&r.stage).to_string(),
            operation: r.stage.clone(),
            input_hashes,
            output_hash: r.content_sha256.clone(),
            parent_record_ids,
            tool_name: Some("fcdp".to_string()),
            agent_name: Some("archon-draft".to_string()),
            model: Some(model.to_string()),
            parameters_json: r.detail.clone(),
            timestamp: now.clone(),
            chain_hash: ch.clone(),
        };
        archon_provenance::store::insert_record(&db, &record)
            .with_context(|| format!("insert record {}", r.record_id))?;

        if let Some(ref prev) = prev_artifact {
            // DerivedFrom points derived → source; `archon prov trace <final>` walks it back.
            let edge = ProvenanceEdge::new(&artifact_id, prev, ProvenanceEdgeType::DerivedFrom);
            archon_provenance::store::insert_edge(&db, &edge).with_context(|| "insert edge")?;
        }

        prev_artifact = Some(artifact_id.clone());
        prev_record_id = Some(r.record_id.clone());
        prev_output_hash = Some(r.content_sha256.clone());
        prev_chain_hash = Some(ch);
        final_artifact = artifact_id;
    }

    // ── Corpus-source linkage (FCDP #3 extension) ────────────────────────────────────────────
    // Resolve each surviving quote to its ingested source and cite it from the assembled node,
    // so the draft chain reaches an ingested `source_document` and `archon prov verify` is valid.
    let anchor = assembled_artifact.unwrap_or_else(|| final_artifact.clone());
    let cited = link_cited_sources(&db, &anchor, quotes, final_draft, &now);

    Ok(Some(ImportSummary {
        records: chain.len(),
        final_artifact,
        cited,
    }))
}

/// Resolve the draft's surviving quotes to ingested corpus documents and insert a `Cites` edge
/// from `anchor` to each distinct source. Best-effort throughout: a store/index failure logs and
/// yields no link rather than aborting a completed import. Edge ids are deterministic per
/// (anchor, document) so a re-import upserts instead of duplicating.
fn link_cited_sources(
    db: &cozo::DbInstance,
    anchor: &str,
    quotes: &[(String, String)],
    final_draft: &str,
    now: &str,
) -> Vec<CitedSource> {
    use archon_docs::quote_verify::{self, MatchKind};
    use archon_provenance::record::{ProvenanceEdge, ProvenanceEdgeType};

    if quotes.is_empty() {
        return Vec::new();
    }
    // Ensure the doc relations exist (idempotent); if the corpus was never ingested here, quote
    // resolution just returns nothing.
    let _ = archon_docs::schema::ensure_doc_schema(db);

    let mut cited: std::collections::BTreeMap<String, CitedSource> =
        std::collections::BTreeMap::new();
    // Distinct corpus nodes to cite. Always the source document (guarantees the trace reaches an
    // ingested `source_document`), plus the exact chunk(s) each quote occupies — the trace then
    // also bridges chunk → page → document via the ingestion synthetic edges, so the lineage is
    // recorded at passage + page + bbox granularity, not just book level.
    let mut cite_targets: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (qid, text) in quotes {
        // Only claim a source whose quote actually survived verbatim into the finished draft.
        if text.trim().is_empty() || !final_draft.contains(text) {
            continue;
        }
        let needle = clean_quote(text);
        if needle.chars().count() < 24 {
            continue; // too short to resolve to a single source with confidence
        }
        let loc = match quote_verify::locate_quote(db, &needle, 1) {
            Ok(mut locs) => locs.drain(..).next(),
            Err(e) => {
                eprintln!("archon draft: quote {qid} source-resolve skipped ({e})");
                None
            }
        };
        let Some(loc) = loc else { continue };
        if loc.similarity < CITE_LINK_FLOOR {
            continue;
        }
        let exact = loc.match_kind == MatchKind::Exact;
        cite_targets.insert(loc.document_id.clone());
        for f in &loc.fragments {
            cite_targets.insert(f.chunk_id.clone());
        }
        let entry = cited
            .entry(loc.document_id.clone())
            .or_insert_with(|| CitedSource {
                document_id: loc.document_id.clone(),
                source_path: loc.source_path.clone(),
                page_start: loc.page_start,
                page_end: loc.page_end,
                similarity: loc.similarity,
                exact,
                quote_ids: Vec::new(),
            });
        entry.quote_ids.push(qid.clone());
        entry.page_start = entry.page_start.min(loc.page_start);
        entry.page_end = entry.page_end.max(loc.page_end);
        entry.similarity = entry.similarity.max(loc.similarity);
        entry.exact = entry.exact || exact;
    }

    for target in &cite_targets {
        // Deterministic id per (anchor, target) so a re-import upserts instead of duplicating.
        let edge = ProvenanceEdge {
            edge_id: format!("edge-cite-{anchor}->{target}"),
            from_artifact_id: anchor.to_string(),
            to_artifact_id: target.clone(),
            edge_type: ProvenanceEdgeType::Cites,
            created_at: now.to_string(),
        };
        if let Err(e) = archon_provenance::store::insert_edge(db, &edge) {
            eprintln!("archon draft: citation edge \u{2192} {target} skipped ({e})");
        }
    }

    cited.into_values().collect()
}

/// Zero-sized handler registered as the primary `/draft` slash command.
///
/// Runs the FCDP drafting protocol from inside the TUI. The model is the
/// session's live model (whatever `/model` last set), overridable per-call
/// with `--model`. A draft takes minutes and `CommandHandler::execute` is
/// sync, so the handler only PARSES + stashes a `CommandEffect::RunDraft`;
/// the dispatch site spawns a detached streaming subprocess (mirrors `/diff`,
/// extended for a long-running command). No aliases.
pub(crate) struct DraftHandler;

impl CommandHandler for DraftHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        // Parse: <pack> <workdir> [--model <name>] [--gate-config <path>].
        let mut positional: Vec<&String> = Vec::new();
        let mut model_override: Option<String> = None;
        let mut gate_config: Option<PathBuf> = None;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--model" => model_override = it.next().cloned(),
                "--gate-config" => gate_config = it.next().map(PathBuf::from),
                _ => positional.push(a),
            }
        }

        if positional.len() < 2 {
            ctx.emit(TuiEvent::TextDelta(
                "\nUsage: /draft <pack.json> <workdir> [--model <name>] [--gate-config <path>]\n\
                 Drafts an FCDP section with the session's current model (see /model).\n"
                    .to_string(),
            ));
            return Ok(());
        }

        let pack = PathBuf::from(positional[0]);
        let workdir = PathBuf::from(positional[1]);

        // Model: explicit --model wins; else the session's live model — the
        // same value /model reads/writes, captured here via model_snapshot
        // (build_command_context populates it for /draft too).
        let model = match model_override {
            Some(m) => m,
            None => match ctx.model_snapshot.as_ref() {
                Some(snap) => snap.current_model.clone(),
                None => {
                    ctx.emit(TuiEvent::Error(
                        "DraftHandler: model_snapshot not populated — build_command_context bug"
                            .to_string(),
                    ));
                    return Ok(());
                }
            },
        };

        // cwd: session working dir → subprocess cwd, so relative pack/workdir
        // paths and the provenance store (`<cwd>/.archon`) resolve from the
        // project root the user is working in.
        let cwd = match &ctx.working_dir {
            Some(p) => p.clone(),
            None => {
                ctx.emit(TuiEvent::Error(
                    "DraftHandler: working_dir not populated in CommandContext".to_string(),
                ));
                return Ok(());
            }
        };

        ctx.emit(TuiEvent::TextDelta(format!(
            "\nDrafting {} with {model} \u{2192} {} (streaming progress; this takes a few minutes)\n",
            pack.display(),
            workdir.display(),
        )));
        ctx.pending_effect = Some(CommandEffect::RunDraft {
            pack,
            workdir,
            model,
            gate_config,
            cwd,
        });
        Ok(())
    }

    fn description(&self) -> &str {
        "Draft an FCDP dissertation section with the current model"
    }
}
