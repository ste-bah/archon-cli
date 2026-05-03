//! Cozo-backed renderers for `archon gametheory` inspection commands.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use archon_pipeline::gametheory;
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

struct RunRow {
    run_id: String,
    situation: String,
    started_at: String,
    completed_at: String,
    status: String,
    cost_usd: String,
}

pub(crate) fn load_run_situation(db: &DbInstance, run_id: &str) -> Result<Option<String>> {
    Ok(load_run_row(db, run_id)?.map(|row| row.situation))
}

pub(crate) fn render_list_runs(db: &DbInstance) -> Result<String> {
    gametheory::schema::ensure_gametheory_schema(db)?;
    let rows = run_immutable(
        db,
        "?[run_id, started_at, status, cost] := \
         *gt_runs{run_id, situation, started_at, completed_at, status, cost_usd: cost}",
        Default::default(),
    )?;

    if rows.rows.is_empty() {
        return Ok("No game-theory runs found.\n".to_string());
    }

    let mut out = String::from("Game-Theory Runs\n================\n");
    for row in &rows.rows {
        let run_id = str_col(row, 0);
        let started = str_col(row, 1);
        let status = str_col(row, 2);
        let cost = str_col(row, 3);
        out.push_str(&format!("  {run_id}  {started}  {status}  ${cost}\n"));
    }
    out.push_str(&format!("{} run(s)\n", rows.rows.len()));
    Ok(out)
}

pub(crate) fn render_status(db: &DbInstance, run_id: Option<&str>) -> Result<String> {
    if let Some(run_id) = run_id {
        return render_one_status(db, run_id);
    }

    gametheory::schema::ensure_gametheory_schema(db)?;
    let rows = run_immutable(
        db,
        "?[status] := *gt_runs{run_id, situation, started_at, completed_at, status}",
        Default::default(),
    )?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for row in &rows.rows {
        *counts.entry(str_col(row, 0)).or_insert(0) += 1;
    }

    let mut out = String::from("Game-Theory Status\n==================\n");
    out.push_str(&format!("Total Runs: {}\n", rows.rows.len()));
    for (status, count) in counts {
        out.push_str(&format!("  {status}: {count}\n"));
    }
    Ok(out)
}

pub(crate) fn render_show(db: &DbInstance, run_id: &str) -> Result<String> {
    let Some(run) = load_run_row(db, run_id)? else {
        return Ok(format!("Run '{run_id}' not found.\n"));
    };

    let mut out = String::from("Game-Theory Run Detail\n======================\n");
    out.push_str(&format!("Run ID:       {}\n", run.run_id));
    out.push_str(&format!("Situation:    {}\n", run.situation));
    out.push_str(&format!("Started:      {}\n", run.started_at));
    out.push_str(&format!(
        "Completed:    {}\n",
        blank_dash(&run.completed_at)
    ));
    out.push_str(&format!("Status:       {}\n", run.status));
    out.push_str(&format!("Cost USD:     ${}\n", run.cost_usd));
    out.push_str(&format!(
        "Fingerprint:  {}\n",
        fingerprint_family(db, run_id)?
    ));
    out.push_str(&format!(
        "Specialists:  {}\n",
        count_by_run(db, "gt_specialist_outputs", run_id)?
    ));
    out.push_str(&format!(
        "Sections:     {}\n",
        count_by_run(db, "gt_sections", run_id)?
    ));
    out.push_str(&format!(
        "Final Report: {}\n",
        final_report_words(db, run_id)?
    ));
    Ok(out)
}

pub(crate) fn render_inspect_fingerprint(db: &DbInstance, run_id: &str) -> Result<String> {
    let Some(fp) = load_fingerprint(db, run_id)? else {
        return Ok(format!("No fingerprint found for run '{run_id}'.\n"));
    };

    let mut out = String::from("Game-Theory Fingerprint\n=======================\n");
    out.push_str(&format!("Run ID:          {}\n", fp.run_id));
    out.push_str(&format!("Primary Family:  {}\n", fp.primary_family));
    out.push_str(&format!(
        "Nearest Classic: {}\n",
        fp.nearest_classic.unwrap_or_else(|| "-".into())
    ));
    out.push_str(&axis_line("Cooperation", &fp.cooperation));
    out.push_str(&axis_line("Payoff Sum", &fp.payoff_sum));
    out.push_str(&axis_line("Symmetry", &fp.symmetry));
    out.push_str(&axis_line("Timing", &fp.timing));
    out.push_str(&axis_line("Perfect Info", &fp.perfect_info));
    out.push_str(&axis_line("Complete Info", &fp.complete_info));
    out.push_str(&axis_line("Cardinality", &fp.cardinality));
    out.push_str(&axis_line("Strategy Space", &fp.strategy_space));
    out.push_str(&axis_line("Horizon", &fp.horizon));
    Ok(out)
}

pub(crate) fn render_inspect_routing(db: &DbInstance, run_id: &str) -> Result<String> {
    gametheory::schema::ensure_gametheory_schema(db)?;
    let rows = run_immutable(
        db,
        "?[enabled, skipped, conditions] := \
         *gt_routing_decisions{run_id, fingerprint_id, enabled_specialists_json: enabled, \
         skipped_specialists_json: skipped, evaluated_conditions_json: conditions, created_at}, \
         run_id = $rid",
        param("rid", run_id),
    )?;
    if rows.rows.is_empty() {
        return Ok(format!("No routing decision found for run '{run_id}'.\n"));
    }

    let enabled: Vec<String> = serde_json::from_str(&str_col(&rows.rows[0], 0)).unwrap_or_default();
    let skipped: Vec<(String, String)> =
        serde_json::from_str(&str_col(&rows.rows[0], 1)).unwrap_or_default();
    let conditions: Vec<(String, bool)> =
        serde_json::from_str(&str_col(&rows.rows[0], 2)).unwrap_or_default();

    let mut out = format!("Routing Decision for {run_id}\n==============================\n");
    out.push_str(&format!("Enabled Specialists ({}):\n", enabled.len()));
    for agent in enabled {
        out.push_str(&format!("  - {agent}\n"));
    }
    out.push_str(&format!("Skipped Specialists ({}):\n", skipped.len()));
    for (agent, reason) in skipped {
        out.push_str(&format!("  - {agent}: {reason}\n"));
    }
    out.push_str(&format!("Evaluated Conditions ({}):\n", conditions.len()));
    for (expr, result) in conditions {
        out.push_str(&format!("  [{result}] {expr}\n"));
    }
    Ok(out)
}

pub(crate) fn render_inspect_artifact(db: &DbInstance, artifact_id: &str) -> Result<String> {
    if let Some(run_id) = artifact_id.strip_prefix("fingerprint:") {
        return render_inspect_fingerprint(db, run_id);
    }
    if let Some(run_id) = artifact_id.strip_prefix("routing:") {
        return render_inspect_routing(db, run_id);
    }
    if let Some(run_id) = artifact_id.strip_prefix("final-report:") {
        return render_final_report(db, run_id);
    }
    if let Some(rest) = artifact_id.strip_prefix("specialist:") {
        if let Some((run_id, agent_key)) = rest.split_once(':') {
            return render_specialist(db, run_id, agent_key);
        }
    }
    if let Some(rest) = artifact_id.strip_prefix("section:") {
        if let Some((run_id, section_id)) = rest.split_once(':') {
            return render_section(db, run_id, section_id);
        }
    }
    if load_run_row(db, artifact_id)?.is_some() {
        return render_show(db, artifact_id);
    }
    Ok(format!(
        "Artifact '{artifact_id}' not found.\nSupported formats: <run-id>, fingerprint:<run-id>, routing:<run-id>, specialist:<run-id>:<agent>, section:<run-id>:<section>, final-report:<run-id>\n"
    ))
}

pub(crate) fn render_list_agents(tier: Option<u8>) -> Result<String> {
    let known_tiers: BTreeSet<u8> = gametheory::registry::GAMETHEORY_TIERS
        .iter()
        .map(|tier| tier.id)
        .collect();
    if let Some(tier) = tier {
        anyhow::ensure!(
            known_tiers.contains(&tier),
            "unknown game-theory tier: {tier}"
        );
    }

    let mut agents: Vec<_> = gametheory::registry::GAMETHEORY_AGENTS
        .iter()
        .filter(|agent| tier.map_or(true, |wanted| agent.tier == wanted))
        .collect();
    agents.sort_by_key(|agent| (agent.tier, agent.key));

    let mut out = String::from("Game-Theory Agents\n==================\n");
    if let Some(tier) = tier {
        out.push_str(&format!("Tier Filter: {tier}\n"));
    }
    out.push_str(&format!("Rows: {}\n", agents.len()));
    for agent in agents {
        out.push_str(&format!(
            "  tier={} key={} model={} mandatory={}\n",
            agent.tier, agent.key, agent.model, agent.mandatory
        ));
    }
    Ok(out)
}

fn render_one_status(db: &DbInstance, run_id: &str) -> Result<String> {
    let Some(run) = load_run_row(db, run_id)? else {
        return Ok(format!("Run '{run_id}' not found.\n"));
    };
    Ok(format!(
        "Game-Theory Run Status\n======================\nRun ID:    {}\nStatus:    {}\nStarted:   {}\nCompleted: {}\nCost USD:  ${}\n",
        run.run_id,
        run.status,
        run.started_at,
        blank_dash(&run.completed_at),
        run.cost_usd
    ))
}

fn load_run_row(db: &DbInstance, run_id: &str) -> Result<Option<RunRow>> {
    gametheory::schema::ensure_gametheory_schema(db)?;
    let rows = run_immutable(
        db,
        "?[situation, started_at, completed_at, status, cost] := \
         *gt_runs{run_id, situation, started_at, completed_at, status, cost_usd: cost}, \
         run_id = $rid",
        param("rid", run_id),
    )?;
    Ok(rows.rows.first().map(|row| RunRow {
        run_id: run_id.to_string(),
        situation: str_col(row, 0),
        started_at: str_col(row, 1),
        completed_at: str_col(row, 2),
        status: str_col(row, 3),
        cost_usd: str_col(row, 4),
    }))
}

fn load_fingerprint(
    db: &DbInstance,
    run_id: &str,
) -> Result<Option<gametheory::GameTheoryFingerprint>> {
    gametheory::schema::ensure_gametheory_schema(db)?;
    let rows = run_immutable(
        db,
        "?[fingerprint_json] := *gt_fingerprints{run_id, fingerprint_json, primary_family, created_at}, run_id = $rid",
        param("rid", run_id),
    )?;
    let Some(row) = rows.rows.first() else {
        return Ok(None);
    };
    let fp = serde_json::from_str(&str_col(row, 0))?;
    Ok(Some(fp))
}

fn render_specialist(db: &DbInstance, run_id: &str, agent_key: &str) -> Result<String> {
    gametheory::schema::ensure_gametheory_schema(db)?;
    let rows = run_immutable(
        db,
        "?[output, status, started, completed, duration, cost] := \
         *gt_specialist_outputs{run_id, agent_key, output_json: output, status, \
         started_at: started, completed_at: completed, duration_ms: duration, cost_usd: cost}, \
         run_id = $rid, agent_key = $agent",
        params2("rid", run_id, "agent", agent_key),
    )?;
    if rows.rows.is_empty() {
        return Ok(format!(
            "No specialist output found for {run_id}:{agent_key}.\n"
        ));
    }
    let row = &rows.rows[0];
    Ok(format!(
        "Specialist Output\n=================\nRun ID:     {run_id}\nAgent:      {agent_key}\nStatus:     {}\nStarted:    {}\nCompleted:  {}\nDurationMs: {}\nCost USD:   ${}\n\n{}\n",
        str_col(row, 1),
        blank_dash(&str_col(row, 2)),
        blank_dash(&str_col(row, 3)),
        str_col(row, 4),
        str_col(row, 5),
        str_col(row, 0)
    ))
}

fn render_section(db: &DbInstance, run_id: &str, section_id: &str) -> Result<String> {
    gametheory::schema::ensure_gametheory_schema(db)?;
    let rows = run_immutable(
        db,
        "?[section_type, title, content, sources, created] := \
         *gt_sections{run_id, section_id, section_type, title, content_md: content, \
         source_specialists_json: sources, created_at: created}, \
         run_id = $rid, section_id = $sid",
        params2("rid", run_id, "sid", section_id),
    )?;
    if rows.rows.is_empty() {
        return Ok(format!("No section found for {run_id}:{section_id}.\n"));
    }
    let row = &rows.rows[0];
    Ok(format!(
        "Section Artifact\n================\nRun ID:  {run_id}\nSection: {section_id}\nType:    {}\nTitle:   {}\nSources: {}\nCreated: {}\n\n{}\n",
        str_col(row, 0),
        str_col(row, 1),
        str_col(row, 3),
        str_col(row, 4),
        str_col(row, 2)
    ))
}

fn render_final_report(db: &DbInstance, run_id: &str) -> Result<String> {
    gametheory::schema::ensure_gametheory_schema(db)?;
    let rows = run_immutable(
        db,
        "?[report, created, cost, duration] := \
         *gt_final_reports{run_id, report_md: report, created_at: created, \
         total_cost_usd: cost, total_duration_ms: duration}, run_id = $rid",
        param("rid", run_id),
    )?;
    if rows.rows.is_empty() {
        return Ok(format!("No final report found for run '{run_id}'.\n"));
    }
    let row = &rows.rows[0];
    Ok(format!(
        "Final Report Artifact\n=====================\nRun ID:     {run_id}\nCreated:    {}\nCost USD:   ${}\nDurationMs: {}\nWords:      {}\n\n{}\n",
        str_col(row, 1),
        str_col(row, 2),
        str_col(row, 3),
        str_col(row, 0).split_whitespace().count(),
        str_col(row, 0)
    ))
}

fn fingerprint_family(db: &DbInstance, run_id: &str) -> Result<String> {
    Ok(load_fingerprint(db, run_id)?
        .map(|fp| fp.primary_family)
        .unwrap_or_else(|| "-".to_string()))
}

fn final_report_words(db: &DbInstance, run_id: &str) -> Result<String> {
    gametheory::schema::ensure_gametheory_schema(db)?;
    let rows = run_immutable(
        db,
        "?[report] := *gt_final_reports{run_id, report_md: report, created_at, total_cost_usd, total_duration_ms}, run_id = $rid",
        param("rid", run_id),
    )?;
    Ok(rows
        .rows
        .first()
        .map(|row| format!("{} words", str_col(row, 0).split_whitespace().count()))
        .unwrap_or_else(|| "-".to_string()))
}

fn count_by_run(db: &DbInstance, relation: &str, run_id: &str) -> Result<usize> {
    let script = match relation {
        "gt_specialist_outputs" => {
            "?[count(agent_key)] := *gt_specialist_outputs{run_id, agent_key}, run_id = $rid"
        }
        "gt_sections" => "?[count(section_id)] := *gt_sections{run_id, section_id}, run_id = $rid",
        _ => anyhow::bail!("unsupported relation count: {relation}"),
    };
    let rows = run_immutable(db, script, param("rid", run_id))?;
    Ok(rows
        .rows
        .first()
        .and_then(|row| row[0].get_int())
        .unwrap_or(0) as usize)
}

fn axis_line(label: &str, axis: &gametheory::fingerprint::AxisVerdict) -> String {
    format!(
        "{label}: {} ({}) - {}\n",
        axis.value, axis.confidence, axis.rationale
    )
}

fn blank_dash(value: &str) -> String {
    if value.trim().is_empty() {
        "-".to_string()
    } else {
        value.to_string()
    }
}

fn str_col(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

fn run_immutable(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
) -> Result<NamedRows> {
    db.run_script(script, params, ScriptMutability::Immutable)
        .map_err(|e| anyhow::anyhow!("cozo query failed: {e}"))
}

fn param(key: &str, value: &str) -> BTreeMap<String, DataValue> {
    let mut params = BTreeMap::new();
    params.insert(key.to_string(), DataValue::from(value));
    params
}

fn params2(k1: &str, v1: &str, k2: &str, v2: &str) -> BTreeMap<String, DataValue> {
    let mut params = param(k1, v1);
    params.insert(k2.to_string(), DataValue::from(v2));
    params
}
