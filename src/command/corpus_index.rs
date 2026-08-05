//! `archon corpus-index` — the claim/clause-level corpus index store (index-overhaul C1).
//!
//! Subcommands: `ensure-schema` (create the corpus_* relations, idempotent), `status`
//! (row counts + import audit trail), `validate` (dry-run a JSONL intermediate against
//! the relation contract — exits non-zero on any invalid record), and `import` (validate
//! + batched write, with per-record quarantine to a sidecar file; NOTHING is silently
//! dropped — every rejected record lands in the quarantine with its reason).

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use archon_knowledge::corpus::{self, spec_for_kind};
use cozo::DbInstance;

use crate::cli_args::CorpusIndexAction;

fn db_path() -> PathBuf {
    crate::command::store_paths::evidence_db_path(&["ARCHON_DOCS_DB_PATH"])
}

fn open_db() -> Result<(Arc<DbInstance>, PathBuf)> {
    let path = db_path();
    let db = archon_docs::acquire_docs_db(path.clone())?;
    corpus::ensure_corpus_schema(&db).map_err(|e| anyhow!("ensure corpus schema: {e}"))?;
    Ok((db, path))
}

pub async fn handle_corpus_index_command(action: CorpusIndexAction) -> Result<()> {
    match action {
        CorpusIndexAction::EnsureSchema => {
            let (db, path) = open_db()?;
            println!(
                "corpus schema ensured ({}) at {}",
                corpus::CORPUS_SCHEMA_VERSION,
                path.display()
            );
            for rel in [
                "corpus_sources",
                "corpus_clauses",
                "corpus_claims",
                "corpus_edges",
                "corpus_tensions",
                "corpus_terms",
                "corpus_groups",
                "corpus_imports",
            ] {
                let n = corpus::count_rows(&db, rel).map_err(|e| anyhow!("{e}"))?;
                println!("  {rel:<18} {n} rows");
            }
            Ok(())
        }
        CorpusIndexAction::Status => {
            let (db, path) = open_db()?;
            println!("corpus index @ {}", path.display());
            let mut total = 0i64;
            for rel in [
                "corpus_sources",
                "corpus_clauses",
                "corpus_claims",
                "corpus_edges",
                "corpus_tensions",
                "corpus_terms",
                "corpus_groups",
            ] {
                let n = corpus::count_rows(&db, rel).map_err(|e| anyhow!("{e}"))?;
                total += n;
                println!("  {rel:<18} {n} rows");
            }
            println!("  {:<18} {} rows", "TOTAL", total);
            println!(
                "  {:<18} {} imports recorded",
                "corpus_imports",
                corpus::count_rows(&db, "corpus_imports").map_err(|e| anyhow!("{e}"))?
            );
            Ok(())
        }
        CorpusIndexAction::Validate { kind, file } => {
            let (_n_ok, errs) = validate_file(&kind, &file)?;
            if errs.is_empty() {
                println!("validate: all records valid ({kind}, {})", file.display());
                Ok(())
            } else {
                for (line, e) in errs.iter().take(20) {
                    eprintln!("  line {line}: {e}");
                }
                bail!(
                    "validate: {} invalid record(s) in {}",
                    errs.len(),
                    file.display()
                );
            }
        }
        CorpusIndexAction::Import {
            kind,
            file,
            quarantine,
            dry_run,
            verify_quotes,
            no_verify_quotes,
        } => {
            // Tier-2 gate is DEFAULT for clauses: explicit --verify-quotes on a non-clauses
            // kind is rejected below; --no-verify-quotes opts out for clauses.
            let gate = (kind == "clauses" && !no_verify_quotes) || verify_quotes;
            import_file(&kind, &file, quarantine.as_deref(), dry_run, gate).await
        }
        CorpusIndexAction::Probes => {
            // E0 (index-overhaul Phase E): archon's ingestion was never covered by a
            // verified audit workflow — every archive figure was a spot measurement.
            // This produces the per-document table the E1–E4 steps must cite.
            let (db, path) = open_db()?;
            let run = |script: &str| -> Result<cozo::NamedRows> {
                db.run_script(
                    script,
                    Default::default(),
                    cozo::ScriptMutability::Immutable,
                )
                .map_err(|e| anyhow!("probe query failed: {e}"))
            };
            let srcs =
                run("?[document_id, source_path] := *doc_sources{document_id, source_path}")?;
            let mut path_of = std::collections::HashMap::new();
            for r in &srcs.rows {
                path_of.insert(
                    r[0].get_str().unwrap_or_default().to_string(),
                    r[1].get_str()
                        .unwrap_or_default()
                        .rsplit('/')
                        .next()
                        .unwrap_or("")
                        .to_string(),
                );
            }
            let spatial = run("?[chunk_id] := *doc_chunk_spatial{chunk_id}")?;
            let spatial_ids: std::collections::HashSet<String> = spatial
                .rows
                .iter()
                .filter_map(|r| r[0].get_str().map(str::to_string))
                .collect();
            // duplicate page-text hashes per document (the audited identical-pages class)
            let pages =
                run("?[document_id, text_hash] := *doc_pages{page_id, document_id, text_hash}")?;
            let mut page_hashes: std::collections::HashMap<String, Vec<String>> =
                Default::default();
            for r in &pages.rows {
                page_hashes
                    .entry(r[0].get_str().unwrap_or_default().to_string())
                    .or_default()
                    .push(r[1].get_str().unwrap_or_default().to_string());
            }
            let mut dup_docs = Vec::new();
            for (doc, hs) in &page_hashes {
                let real: Vec<&String> = hs
                    .iter()
                    .filter(|h| !h.is_empty() && *h != "none")
                    .collect();
                let distinct: std::collections::HashSet<&&String> = real.iter().collect();
                if real.len() >= 3 && distinct.len() < real.len() {
                    dup_docs.push(serde_json::json!({
                        "document_id": doc,
                        "pages_with_text": real.len(),
                        "distinct_hashes": distinct.len(),
                    }));
                }
            }
            // sentence layer (S2/S3) stats per chunk → per doc
            let sent = run("?[chunk_id, count(sentence_idx), sum(has_bbox)] := \
                 *doc_chunk_sentences{chunk_id, sentence_idx, bbox}, \
                 has_bbox = if(bbox == '', 0, 1)")
            .unwrap_or_else(|_| cozo::NamedRows::new(vec![], vec![]));
            let mut sent_of: std::collections::HashMap<String, (i64, i64)> = Default::default();
            for r in &sent.rows {
                sent_of.insert(
                    r[0].get_str().unwrap_or_default().to_string(),
                    (
                        r[1].get_int().unwrap_or(0),
                        r[2].get_int()
                            .unwrap_or(r[2].get_float().unwrap_or(0.0) as i64),
                    ),
                );
            }
            let locs =
                run("?[document_id, count(locator_id)] := *doc_locators{locator_id, document_id}")?;
            let mut loc_of = std::collections::HashMap::new();
            for r in &locs.rows {
                loc_of.insert(
                    r[0].get_str().unwrap_or_default().to_string(),
                    r[1].get_int().unwrap_or(0),
                );
            }
            const LIG: &[&str] = &[
                "benefcial",
                "signifcant",
                "difcult",
                "specifcally",
                "confdence",
                "benefts",
                "signifcance",
                "efcient",
                "infuence",
                "refect ",
                "frst ",
                "fnd ",
            ];
            const OCR: &[&str] = &["pnractical", "consiaered", "Involvernent", "1tsely"];
            let chunks = run(
                "?[document_id, chunk_id, content] := *doc_chunks{chunk_id, document_id, content}",
            )?;
            #[derive(Default)]
            struct Row {
                chunks: i64,
                spatial: i64,
                lig: i64,
                ocr: i64,
                sentences: i64,
                sent_bbox: i64,
            }
            let mut per: std::collections::BTreeMap<String, Row> = Default::default();
            for r in &chunks.rows {
                let doc = r[0].get_str().unwrap_or_default().to_string();
                let cid = r[1].get_str().unwrap_or_default();
                let content = r[2].get_str().unwrap_or_default();
                let e = per.entry(doc).or_default();
                e.chunks += 1;
                if spatial_ids.contains(cid) {
                    e.spatial += 1;
                }
                if let Some(&(n, nb)) = sent_of.get(cid) {
                    e.sentences += n;
                    e.sent_bbox += nb;
                }
                e.lig += LIG
                    .iter()
                    .map(|p| content.matches(p).count() as i64)
                    .sum::<i64>();
                e.ocr += OCR
                    .iter()
                    .map(|p| content.matches(p).count() as i64)
                    .sum::<i64>();
            }
            let mut out = Vec::new();
            let (mut tot_c, mut tot_s, mut docs_lig, mut docs_full) = (0i64, 0i64, 0usize, 0usize);
            for (doc, row) in &per {
                tot_c += row.chunks;
                tot_s += row.spatial;
                if row.lig > 0 {
                    docs_lig += 1;
                }
                if row.spatial == row.chunks {
                    docs_full += 1;
                }
                out.push(serde_json::json!({
                    "document_id": doc,
                    "source": path_of.get(doc).cloned().unwrap_or_default(),
                    "chunks": row.chunks,
                    "spatial_rows": row.spatial,
                    "spatial_coverage": (row.spatial as f64 / row.chunks.max(1) as f64 * 1000.0).round() / 1000.0,
                    "locators": loc_of.get(doc).copied().unwrap_or(0),
                    "ligature_hits": row.lig,
                    "ocr_damage_hits": row.ocr,
                    "sentences": row.sentences,
                    "sentences_with_bbox": row.sent_bbox,
                }));
            }
            let report = serde_json::json!({
                "generated_by": "archon corpus-index probes (E0)",
                "documents": out.len(),
                "total_chunks": tot_c,
                "total_spatial_rows": tot_s,
                "spatial_coverage_overall": (tot_s as f64 / tot_c.max(1) as f64 * 1000.0).round() / 1000.0,
                "docs_with_full_spatial": docs_full,
                "docs_with_ligature_dropout": docs_lig,
                "docs_with_duplicate_page_hashes": dup_docs,
                "per_document": out,
            });
            let dest = path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("corpus-import/corpus-probes.json");
            std::fs::write(&dest, serde_json::to_string_pretty(&report)?)?;
            println!(
                "documents: {} | chunks: {tot_c} | spatial coverage: {:.1}% | full-spatial docs: {} | ligature-affected docs: {}",
                out.len(),
                tot_s as f64 / tot_c.max(1) as f64 * 100.0,
                docs_full,
                docs_lig
            );
            println!("written: {}", dest.display());
            Ok(())
        }
        CorpusIndexAction::Dump { kind, entry } => {
            let spec = spec_for_kind(&kind).ok_or_else(|| {
                anyhow!(
                    "unknown kind '{kind}' (expected one of {:?})",
                    corpus::ALL_KINDS
                )
            })?;
            let (db, _path) = open_db()?;
            let names: Vec<&str> = spec.columns.iter().map(|(c, _)| *c).collect();
            let head = names.join(", ");
            let has_entry = names.contains(&"entry_id");
            let mut params = std::collections::BTreeMap::new();
            let script = if let Some(e) = entry.as_deref().filter(|_| has_entry) {
                params.insert("e".to_string(), cozo::DataValue::from(e));
                format!("?[{head}] := *{}{{ {head} }}, entry_id = $e", spec.relation)
            } else {
                format!("?[{head}] := *{}{{ {head} }}", spec.relation)
            };
            let out = db
                .run_script(&script, params, cozo::ScriptMutability::Immutable)
                .map_err(|e| anyhow!("dump {}: {e}", spec.relation))?;
            let stdout = std::io::stdout();
            let mut w = std::io::BufWriter::new(stdout.lock());
            use std::io::Write;
            for row in &out.rows {
                let mut obj = serde_json::Map::new();
                for (i, (col, _)) in spec.columns.iter().enumerate() {
                    let v = &row[i];
                    let jv = if let Some(s) = v.get_str() {
                        serde_json::Value::String(s.to_string())
                    } else if let Some(n) = v.get_int() {
                        serde_json::Value::from(n)
                    } else if let Some(b) = v.get_bool() {
                        serde_json::Value::from(b)
                    } else {
                        serde_json::Value::String(format!("{v:?}"))
                    };
                    obj.insert((*col).to_string(), jv);
                }
                writeln!(w, "{}", serde_json::Value::Object(obj))?;
            }
            Ok(())
        }
        CorpusIndexAction::Show { kind, id } => {
            let spec = spec_for_kind(&kind).ok_or_else(|| {
                anyhow!(
                    "unknown kind '{kind}' (expected one of {:?})",
                    corpus::ALL_KINDS
                )
            })?;
            let (db, _path) = open_db()?;
            let names: Vec<&str> = spec.columns.iter().map(|(c, _)| *c).collect();
            let head = names.join(", ");
            let script = format!(
                "?[{head}] := *{}{{ {head} }}, {} = $id",
                spec.relation, spec.key
            );
            let mut params = std::collections::BTreeMap::new();
            params.insert("id".to_string(), cozo::DataValue::from(id.as_str()));
            let out = db
                .run_script(&script, params, cozo::ScriptMutability::Immutable)
                .map_err(|e| anyhow!("query {}: {e}", spec.relation))?;
            let Some(row) = out.rows.first() else {
                bail!("{} '{}' not found", spec.relation, id);
            };
            let mut obj = serde_json::Map::new();
            for (i, (col, _)) in spec.columns.iter().enumerate() {
                let v = &row[i];
                let jv = if let Some(s) = v.get_str() {
                    serde_json::Value::String(s.to_string())
                } else if let Some(n) = v.get_int() {
                    serde_json::Value::from(n)
                } else if let Some(b) = v.get_bool() {
                    serde_json::Value::from(b)
                } else {
                    serde_json::Value::String(format!("{v:?}"))
                };
                obj.insert((*col).to_string(), jv);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::Value::Object(obj))?
            );
            Ok(())
        }
        CorpusIndexAction::Remove {
            kind,
            id,
            ids_file,
            yes,
        } => {
            let spec = spec_for_kind(&kind).ok_or_else(|| {
                anyhow!(
                    "unknown kind '{kind}' (expected one of {:?})",
                    corpus::ALL_KINDS
                )
            })?;
            let ids: Vec<String> = match (&id, &ids_file) {
                (Some(one), None) => vec![one.clone()],
                (None, Some(f)) => std::fs::read_to_string(f)
                    .with_context(|| format!("read {}", f.display()))?
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(String::from)
                    .collect(),
                (Some(_), Some(_)) => bail!("pass either an id or --ids-file, not both"),
                (None, None) => bail!("pass an id or --ids-file"),
            };
            if ids.is_empty() {
                bail!("no ids to remove");
            }
            let (db, path) = open_db()?;
            // verify-before-remove: :rm on a missing key is a silent no-op, so
            // existence is checked here and any miss aborts the whole batch.
            let script = format!(
                "?[{k}] := *{}{{ {k} }}, is_in({k}, $ids)",
                spec.relation,
                k = spec.key
            );
            let mut params = std::collections::BTreeMap::new();
            params.insert(
                "ids".to_string(),
                cozo::DataValue::List(
                    ids.iter()
                        .map(|i| cozo::DataValue::from(i.as_str()))
                        .collect(),
                ),
            );
            let out = db
                .run_script(&script, params, cozo::ScriptMutability::Immutable)
                .map_err(|e| anyhow!("query {}: {e}", spec.relation))?;
            let found: std::collections::HashSet<String> = out
                .rows
                .iter()
                .filter_map(|r| r[0].get_str().map(String::from))
                .collect();
            let missing: Vec<&String> = ids.iter().filter(|i| !found.contains(*i)).collect();
            if !missing.is_empty() {
                bail!(
                    "{} of {} id(s) not found in {} (first: {:?}) — aborting, nothing removed",
                    missing.len(),
                    ids.len(),
                    spec.relation,
                    missing.iter().take(5).collect::<Vec<_>>()
                );
            }
            if !yes {
                bail!(
                    "would remove {} record(s) from {} — pass -y to remove",
                    ids.len(),
                    spec.relation
                );
            }
            let started = chrono::Utc::now().to_rfc3339();
            let n =
                corpus::remove_rows(&db, Some(&path), spec, &ids).map_err(|e| anyhow!("{e}"))?;
            let finished = chrono::Utc::now().to_rfc3339();
            let what = if ids.len() == 1 {
                format!("remove:{}", ids[0])
            } else {
                format!("remove-batch:{} ids", ids.len())
            };
            let rm_id = format!(
                "rm-{}-{}",
                kind,
                &sha256_hex(&format!("{what}|{started}"))[..12]
            );
            corpus::record_import(
                &db,
                Some(&path),
                &rm_id,
                &kind,
                &what,
                n as i64,
                0,
                0,
                "",
                &started,
                &finished,
            )
            .map_err(|e| anyhow!("{e}"))?;
            println!("removed {} record(s) from {} [{rm_id}]", n, spec.relation);
            Ok(())
        }
    }
}

/// Parse the intermediate: JSONL (one object per line) or a single JSON array.
fn read_records(file: &Path) -> Result<Vec<(usize, serde_json::Value)>> {
    let f = std::fs::File::open(file).with_context(|| format!("open {}", file.display()))?;
    let mut first = String::new();
    let mut reader = std::io::BufReader::new(f);
    reader.read_line(&mut first)?;
    let trimmed = first.trim_start();
    if trimmed.starts_with('[') {
        // whole-file JSON array
        let text = std::fs::read_to_string(file)?;
        let arr: Vec<serde_json::Value> =
            serde_json::from_str(&text).with_context(|| "parse JSON array")?;
        return Ok(arr
            .into_iter()
            .enumerate()
            .map(|(i, v)| (i + 1, v))
            .collect());
    }
    // JSONL
    let mut out = Vec::new();
    let mut push = |lineno: usize, s: &str| -> Result<()> {
        let t = s.trim();
        if t.is_empty() {
            return Ok(());
        }
        let v: serde_json::Value =
            serde_json::from_str(t).with_context(|| format!("line {lineno}: invalid JSON"))?;
        out.push((lineno, v));
        Ok(())
    };
    push(1, &first)?;
    for (i, line) in reader.lines().enumerate() {
        push(i + 2, &line?)?;
    }
    Ok(out)
}

fn validate_file(kind: &str, file: &Path) -> Result<(usize, Vec<(usize, String)>)> {
    let spec = spec_for_kind(kind).ok_or_else(|| {
        anyhow!(
            "unknown kind '{kind}' (expected one of {:?})",
            corpus::ALL_KINDS
        )
    })?;
    let records = read_records(file)?;
    let mut ok = 0usize;
    let mut errs = Vec::new();
    for (line, obj) in &records {
        match corpus::row_from_json(spec, obj) {
            Ok(_) => ok += 1,
            Err(e) => errs.push((*line, e)),
        }
    }
    Ok((ok, errs))
}

async fn import_file(
    kind: &str,
    file: &Path,
    quarantine: Option<&Path>,
    dry_run: bool,
    verify_quotes: bool,
) -> Result<()> {
    let spec = spec_for_kind(kind).ok_or_else(|| {
        anyhow!(
            "unknown kind '{kind}' (expected one of {:?})",
            corpus::ALL_KINDS
        )
    })?;
    if verify_quotes && kind != "clauses" {
        anyhow::bail!("--verify-quotes applies to kind 'clauses' only (got '{kind}')");
    }
    let started = chrono::Utc::now().to_rfc3339();
    let mut records = read_records(file)?;
    let rows_in = records.len();

    // Tier-2 quote gate (mandatory verification, 2026-08-05): anchored clause rows
    // must verify their quote against their pinned document BEFORE schema import —
    // failures join the quarantine with a reason. Entries are born verified.
    let mut bad: Vec<serde_json::Value> = Vec::new();
    let mut db_and_path = if verify_quotes || !dry_run {
        Some(open_db()?)
    } else {
        None
    };
    if verify_quotes {
        let (db, _) = db_and_path.as_ref().expect("db opened for quote gate");
        let (pass, rejected, stats) = super::corpus_index_verify::gate_clause_records(db, records);
        records = pass;
        for (line, obj, reason) in rejected {
            bad.push(serde_json::json!({
                "line": line,
                "error": format!("quote-gate: {reason}"),
                "record": obj,
            }));
        }
        println!("{}", stats.summary());
    }

    let mut good = Vec::new();
    for (line, obj) in records {
        match corpus::row_from_json(spec, &obj) {
            Ok(row) => good.push(row),
            Err(e) => bad.push(serde_json::json!({"line": line, "error": e, "record": obj})),
        }
    }

    if dry_run {
        println!(
            "DRY RUN {kind}: {} valid, {} would be quarantined (of {rows_in})",
            good.len(),
            bad.len()
        );
        return Ok(());
    }

    let (db, path) = db_and_path.take().expect("db opened for import write");
    let mut written = 0usize;
    for batch in good.chunks(1000) {
        written +=
            corpus::put_rows(&db, Some(&path), spec, batch.to_vec()).map_err(|e| anyhow!("{e}"))?;
    }

    // Quarantine: every rejected record is WRITTEN with its reason — never dropped.
    let qpath = if bad.is_empty() {
        String::new()
    } else {
        let qp: PathBuf = quarantine
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| file.with_extension("quarantine.jsonl"));
        let mut body = String::new();
        for b in &bad {
            body.push_str(&b.to_string());
            body.push('\n');
        }
        std::fs::write(&qp, body).with_context(|| format!("write quarantine {}", qp.display()))?;
        qp.display().to_string()
    };

    let finished = chrono::Utc::now().to_rfc3339();
    let import_id = format!(
        "imp-{}-{}",
        kind,
        &sha256_hex(&format!("{}|{}|{}", file.display(), started, rows_in))[..12]
    );
    corpus::record_import(
        &db,
        Some(&path),
        &import_id,
        kind,
        &file.display().to_string(),
        rows_in as i64,
        written as i64,
        bad.len() as i64,
        &qpath,
        &started,
        &finished,
    )
    .map_err(|e| anyhow!("{e}"))?;

    println!(
        "import {kind}: {written} written, {} quarantined (of {rows_in}) [{import_id}]",
        bad.len()
    );
    if !bad.is_empty() {
        eprintln!(
            "  quarantine: {qpath} (first error: {})",
            bad.first()
                .and_then(|b| b.get("error"))
                .cloned()
                .unwrap_or_default()
        );
    }
    Ok(())
}

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}
