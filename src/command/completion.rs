//! CLI handler for `archon completion` commands — TSPEC §10.
//!
//! Subcommands: inspect, claims, evidence, incidents, verify.

use anyhow::Result;
use cozo::DbInstance;

use crate::cli_args::CompletionAction;

/// Dispatch the completion subcommand.
pub async fn handle_completion(action: &CompletionAction) -> Result<()> {
    match action {
        CompletionAction::Inspect { run_id, .. } => handle_inspect(run_id),
        CompletionAction::Claims { run_id } => handle_claims(run_id),
        CompletionAction::Evidence { run_id } => handle_evidence(run_id),
        CompletionAction::Incidents => handle_incidents(),
        CompletionAction::Verify { run_id, task_type } => handle_verify(run_id, task_type).await,
    }
}

// ── inspect ────────────────────────────────────────────────────────────────────

fn handle_inspect(run_id: &str) -> Result<()> {
    let db = open_db()?;
    archon_completion::schema::ensure_completion_schema(&db)
        .map_err(|e| anyhow::anyhow!("schema init failed: {e}"))?;

    // Query all three sources for this run
    let claims = archon_completion::store::get_completion_claims_by_run(&db, run_id)
        .map_err(|e| anyhow::anyhow!("query claims failed: {e}"))?;
    let evidence = archon_completion::store::get_evidence_by_run(&db, run_id)
        .map_err(|e| anyhow::anyhow!("query evidence failed: {e}"))?;

    if claims.is_empty() && evidence.is_empty() {
        println!("No completion data found for run '{run_id}'.");
        println!("Run `archon completion verify {run_id}` to generate a report.");
        return Ok(());
    }

    println!("Completion Integrity — Run: {run_id}");
    println!("======================================");
    println!();

    // Claims
    println!("Claims ({})", claims.len());
    println!("-------");
    if claims.is_empty() {
        println!("(none)");
    } else {
        for c in &claims {
            let status = if c.verified { "VERIFIED" } else { "BLOCKED" };
            println!("  [{status}] {:?} — \"{}\"", c.claim_kind, c.claim_text);
        }
    }
    println!();

    // Evidence
    println!("Evidence ({})", evidence.len());
    println!("--------");
    if evidence.is_empty() {
        println!("(none)");
    } else {
        for ev in &evidence {
            println!(
                "  [{:?}] {:?} from {}",
                ev.status, ev.evidence_kind, ev.producer
            );
            if let Some(ref summary) = ev.stdout_summary {
                println!("         {summary}");
            }
        }
    }
    println!();

    // Report summary (if persisted)
    let result = db.run_script(
        "?[report_id, final_state, calibrated_summary, created_at] \
         := *completion_reports{report_id, run_id, final_state, claims_json, \
         evidence_json, failed_gates_json, unverified_claims_json, \
         calibrated_summary, provenance_record_id, created_at}, run_id = $rid",
        {
            let mut p = std::collections::BTreeMap::new();
            p.insert("rid".into(), cozo::DataValue::from(run_id));
            p
        },
        cozo::ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("query completion_reports failed: {e}"))?;

    if !result.rows.is_empty() {
        let state = result.rows[0][1].get_str().unwrap_or("?");
        let summary = result.rows[0][2].get_str().unwrap_or("");
        let created = result.rows[0][3].get_str().unwrap_or("?");
        println!("Report — Final State: {state}  Created: {created}");
        println!("------");
        println!("{summary}");
    }

    Ok(())
}

// ── claims ─────────────────────────────────────────────────────────────────────

fn handle_claims(run_id: &str) -> Result<()> {
    let db = open_db()?;
    archon_completion::schema::ensure_completion_schema(&db)
        .map_err(|e| anyhow::anyhow!("schema init failed: {e}"))?;

    let claims = archon_completion::store::get_completion_claims_by_run(&db, run_id)
        .map_err(|e| anyhow::anyhow!("query claims failed: {e}"))?;

    if claims.is_empty() {
        println!("No claims found for run '{run_id}'.");
        return Ok(());
    }

    println!("Claims for {run_id} ({})", claims.len());
    for c in &claims {
        let status = if c.verified { "VERIFIED" } else { "BLOCKED" };
        println!(
            "  {claim_id} [{status}] {kind:?} — \"{text}\"",
            claim_id = c.claim_id,
            kind = c.claim_kind,
            text = c.claim_text,
        );
    }
    Ok(())
}

// ── evidence ───────────────────────────────────────────────────────────────────

fn handle_evidence(run_id: &str) -> Result<()> {
    let db = open_db()?;
    archon_completion::schema::ensure_completion_schema(&db)
        .map_err(|e| anyhow::anyhow!("schema init failed: {e}"))?;

    let evidence = archon_completion::store::get_evidence_by_run(&db, run_id)
        .map_err(|e| anyhow::anyhow!("query evidence failed: {e}"))?;

    if evidence.is_empty() {
        println!("No evidence found for run '{run_id}'.");
        return Ok(());
    }

    println!("Evidence for {run_id} ({})", evidence.len());
    for ev in &evidence {
        println!(
            "  {eid} [{status:?}] {kind:?} from {producer}",
            eid = ev.evidence_id,
            status = ev.status,
            kind = ev.evidence_kind,
            producer = ev.producer,
        );
    }
    Ok(())
}

// ── incidents ──────────────────────────────────────────────────────────────────

fn handle_incidents() -> Result<()> {
    let db = open_db()?;
    archon_completion::schema::ensure_completion_schema(&db)
        .map_err(|e| anyhow::anyhow!("schema init failed: {e}"))?;

    let incidents = archon_completion::store::get_all_incidents(&db)
        .map_err(|e| anyhow::anyhow!("query incidents failed: {e}"))?;

    if incidents.is_empty() {
        println!("No false-completion incidents recorded.");
        return Ok(());
    }

    println!("False-Completion Incidents ({})", incidents.len());
    for inc in &incidents {
        println!(
            "  {iid} [{severity:?}] run={rid} agent={agent:?} model={model:?}",
            iid = inc.incident_id,
            severity = inc.severity,
            rid = inc.run_id,
            agent = inc.agent_key,
            model = inc.model,
        );
        println!(
            "    Claimed: \"{}\"  Actual: {:?}",
            inc.claimed_state, inc.actual_state
        );
        if !inc.missing_evidence.is_empty() {
            println!("    Missing evidence: {:?}", inc.missing_evidence);
        }
        if let Some(ref correction) = inc.user_correction {
            println!("    User correction: {correction}");
        }
    }
    Ok(())
}

// ── verify ─────────────────────────────────────────────────────────────────────

async fn handle_verify(run_id: &str, task_type: &str) -> Result<()> {
    let db = open_db()?;

    // Read output text from stdin if available
    let output_text = if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        std::io::read_to_string(std::io::stdin())
            .unwrap_or_default()
            .trim()
            .to_string()
    } else {
        println!("No output text provided via stdin.");
        println!("Usage: echo \"<output text>\" | archon completion verify <run_id>");
        println!();
        anyhow::bail!("output text required via stdin for verification");
    };

    if output_text.is_empty() {
        anyhow::bail!("output text is empty");
    }

    let report = archon_completion::check_completion(&db, run_id, &output_text, task_type)
        .await
        .map_err(|e| anyhow::anyhow!("completion check failed: {e}"))?;

    println!(
        "{report_id} [{state:?}]",
        report_id = report.report_id,
        state = report.final_state,
    );
    if !report.failed_gates.is_empty() {
        println!("Failed gates: {}", report.failed_gates.join(", "));
    }
    if !report.unverified_claims.is_empty() {
        println!("Unverified claims: {}", report.unverified_claims.join(", "));
    }
    println!();

    match report.final_state {
        archon_completion::models::CompletionState::Verified => {
            std::process::exit(0);
        }
        _ => {
            std::process::exit(1);
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn open_db() -> Result<DbInstance> {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".local/share"))
        .join("archon");
    std::fs::create_dir_all(&data_dir)?;
    let path = data_dir.join("archon-data.db");
    let path_str = path.to_string_lossy().to_string();
    DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("Failed to open completion store at {path_str}: {e}"))
}
