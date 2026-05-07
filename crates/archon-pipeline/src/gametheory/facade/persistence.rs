use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use chrono::Utc;
use cozo::DbInstance;

use super::super::final_stage;
use super::super::quality;
use super::super::routing::RoutingDecision;
use super::super::schema::ensure_gametheory_schema;
use super::costs::agent_tier;

pub(super) fn persist_routing_decision(db: &DbInstance, rd: &RoutingDecision) -> Result<()> {
    ensure_gametheory_schema(db)?;

    let enabled_json =
        serde_json::to_string(&rd.enabled_specialists).unwrap_or_else(|_| "[]".into());
    let skipped_json =
        serde_json::to_string(&rd.skipped_specialists).unwrap_or_else(|_| "[]".into());
    let conditions_json =
        serde_json::to_string(&rd.evaluated_conditions).unwrap_or_else(|_| "[]".into());

    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(rd.run_id.as_str()));
    params.insert(
        "fid".into(),
        cozo::DataValue::from(rd.fingerprint_id.as_str()),
    );
    params.insert("en".into(), cozo::DataValue::from(enabled_json.as_str()));
    params.insert("sk".into(), cozo::DataValue::from(skipped_json.as_str()));
    params.insert("ec".into(), cozo::DataValue::from(conditions_json.as_str()));
    params.insert("ca".into(), cozo::DataValue::from(rd.created_at.as_str()));

    db.run_script(
        "?[run_id, fingerprint_id, enabled_specialists_json, skipped_specialists_json, \
         evaluated_conditions_json, created_at] \
         <- [[$rid, $fid, $en, $sk, $ec, $ca]] \
         :put gt_routing_decisions { run_id => fingerprint_id, enabled_specialists_json, \
         skipped_specialists_json, evaluated_conditions_json, created_at }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("persist gt_routing_decisions failed: {e}"))?;
    Ok(())
}

pub(super) fn persist_specialist_outputs(
    db: &DbInstance,
    run_id: &str,
    outputs: &HashMap<String, String>,
    costs_usd: &HashMap<String, f64>,
) -> Result<()> {
    ensure_gametheory_schema(db)?;
    let now = Utc::now().to_rfc3339();

    for (agent_key, output) in outputs {
        let mut params = BTreeMap::new();
        params.insert("rid".into(), cozo::DataValue::from(run_id));
        params.insert("ak".into(), cozo::DataValue::from(agent_key.as_str()));
        params.insert("out".into(), cozo::DataValue::from(output.as_str()));
        params.insert("status".into(), cozo::DataValue::from("completed"));
        params.insert("started".into(), cozo::DataValue::from(now.as_str()));
        params.insert("completed".into(), cozo::DataValue::from(now.as_str()));
        params.insert("duration".into(), cozo::DataValue::from("0"));
        let cost = format!("{:.6}", costs_usd.get(agent_key).copied().unwrap_or(0.0));
        params.insert("cost".into(), cozo::DataValue::from(cost.as_str()));

        db.run_script(
            "?[run_id, agent_key, output_json, status, started_at, completed_at, \
             duration_ms, cost_usd] <- [[$rid, $ak, $out, $status, $started, \
             $completed, $duration, $cost]] \
             :put gt_specialist_outputs { run_id, agent_key => output_json, status, \
             started_at, completed_at, duration_ms, cost_usd }",
            params,
            cozo::ScriptMutability::Mutable,
        )
        .map_err(|e| anyhow::anyhow!("persist gt_specialist_outputs failed: {e}"))?;
        persist_run_checkpoint(
            db,
            run_id,
            &format!("specialist:{agent_key}"),
            "specialist",
            "completed",
            serde_json::json!({
                "agent_key": agent_key,
                "tier": agent_tier(agent_key),
                "cost_usd": cost,
            }),
        )?;
    }
    Ok(())
}

pub(super) fn persist_specialist_failure(
    db: &DbInstance,
    run_id: &str,
    agent_key: &str,
    message: &str,
) -> Result<()> {
    ensure_gametheory_schema(db)?;
    let now = Utc::now().to_rfc3339();
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("ak".into(), cozo::DataValue::from(agent_key));
    params.insert("out".into(), cozo::DataValue::from(message));
    params.insert("status".into(), cozo::DataValue::from("failed"));
    params.insert("now".into(), cozo::DataValue::from(now.as_str()));

    db.run_script(
        "?[run_id, agent_key, output_json, status, started_at, completed_at, duration_ms, cost_usd] \
         <- [[$rid, $ak, $out, $status, $now, $now, '0', '0.000000']] \
         :put gt_specialist_outputs { run_id, agent_key => output_json, status, \
         started_at, completed_at, duration_ms, cost_usd }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("persist failed gt_specialist_outputs failed: {e}"))?;
    persist_run_checkpoint(
        db,
        run_id,
        &format!("specialist:{agent_key}"),
        "specialist",
        "failed",
        serde_json::json!({
            "agent_key": agent_key,
            "tier": agent_tier(agent_key),
            "message": message,
        }),
    )?;
    Ok(())
}

pub(super) fn persist_sections(
    db: &DbInstance,
    run_id: &str,
    sections: &[final_stage::writer::SectionContent],
) -> Result<()> {
    ensure_gametheory_schema(db)?;

    let now = Utc::now().to_rfc3339();
    for (idx, section) in sections.iter().enumerate() {
        let section_id = format!("sec-{:02}", idx + 1);
        let title = section.section.title();
        let contributors_json = serde_json::to_string(&section.contributors)
            .map_err(|e| anyhow::anyhow!("serialize section contributors failed: {e}"))?;

        let mut params = BTreeMap::new();
        params.insert("rid".into(), cozo::DataValue::from(run_id));
        params.insert("sid".into(), cozo::DataValue::from(section_id.as_str()));
        params.insert("sty".into(), cozo::DataValue::from(title));
        params.insert("stt".into(), cozo::DataValue::from(title));
        params.insert(
            "smd".into(),
            cozo::DataValue::from(section.content.as_str()),
        );
        params.insert(
            "ssj".into(),
            cozo::DataValue::from(contributors_json.as_str()),
        );
        params.insert("ca".into(), cozo::DataValue::from(now.as_str()));

        db.run_script(
            "?[run_id, section_id, section_type, title, content_md, \
             source_specialists_json, created_at] \
             <- [[$rid, $sid, $sty, $stt, $smd, $ssj, $ca]] \
             :put gt_sections { run_id, section_id => section_type, title, \
             content_md, source_specialists_json, created_at }",
            params,
            cozo::ScriptMutability::Mutable,
        )
        .map_err(|e| anyhow::anyhow!("persist gt_sections failed: {e}"))?;
    }
    Ok(())
}

pub(super) fn persist_quality_checks(
    db: &DbInstance,
    run_id: &str,
    quality_results: &HashMap<String, Vec<quality::QualityCheck>>,
) -> Result<()> {
    let created_at = Utc::now().to_rfc3339();
    for (agent_key, checks) in quality_results {
        for check in checks {
            let mut params = BTreeMap::new();
            params.insert("rid".into(), cozo::DataValue::from(run_id));
            params.insert("agent".into(), cozo::DataValue::from(agent_key.as_str()));
            params.insert("gate".into(), cozo::DataValue::from(check.gate_name));
            params.insert(
                "passed".into(),
                cozo::DataValue::from(if check.passed { "true" } else { "false" }),
            );
            params.insert(
                "detail".into(),
                cozo::DataValue::from(check.detail.as_str()),
            );
            params.insert("created".into(), cozo::DataValue::from(created_at.as_str()));

            db.run_script(
                "?[run_id, agent_key, gate_name, passed, detail, created_at] \
                 <- [[$rid, $agent, $gate, $passed, $detail, $created]] \
                 :put gt_quality_checks { run_id, agent_key, gate_name => passed, detail, created_at }",
                params,
                cozo::ScriptMutability::Mutable,
            )
            .map_err(|e| anyhow::anyhow!("persist gt_quality_checks failed: {e}"))?;
        }
    }
    Ok(())
}

pub(super) fn persist_final_report(
    db: &DbInstance,
    run_id: &str,
    report: &str,
    total_cost_usd: f64,
) -> Result<()> {
    ensure_gametheory_schema(db)?;

    let now = Utc::now().to_rfc3339();

    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("rep".into(), cozo::DataValue::from(report));
    params.insert("ca".into(), cozo::DataValue::from(now.as_str()));
    let cost = format!("{total_cost_usd:.6}");
    params.insert("cost".into(), cozo::DataValue::from(cost.as_str()));

    db.run_script(
        "?[run_id, report_md, created_at, total_cost_usd, total_duration_ms] \
         <- [[$rid, $rep, $ca, $cost, '0']] \
         :put gt_final_reports { run_id => report_md, created_at, \
         total_cost_usd, total_duration_ms }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("persist gt_final_reports failed: {e}"))?;
    Ok(())
}

pub(super) fn persist_provenance_edges_for_run<'a>(
    db: &DbInstance,
    run_id: &str,
    specialist_keys: impl IntoIterator<Item = &'a String>,
    sections: &[final_stage::writer::SectionContent],
) -> Result<()> {
    ensure_gametheory_schema(db)?;
    let mut edges = Vec::new();

    let situation = format!("situation:{run_id}");
    let fingerprint = format!("fingerprint:{run_id}");
    let routing = format!("routing:{run_id}");
    let report = format!("report:{run_id}");
    edges.push((situation, fingerprint.clone(), "produced_fingerprint"));
    edges.push((fingerprint.clone(), routing.clone(), "produced_routing"));

    for agent_key in specialist_keys {
        let specialist = specialist_artifact_id(run_id, agent_key);
        edges.push((routing.clone(), specialist, "enabled_specialist"));
    }

    for (idx, section) in sections.iter().enumerate() {
        let section_id = section_artifact_id(run_id, idx);
        for contributor in &section.contributors {
            edges.push((
                specialist_artifact_id(run_id, contributor),
                section_id.clone(),
                "contributed_to_section",
            ));
        }
        edges.push((section_id, report.clone(), "assembled_into_report"));
    }

    for (idx, (from, to, edge_type)) in edges.iter().enumerate() {
        persist_provenance_edge(db, run_id, idx + 1, from, to, edge_type)?;
    }
    Ok(())
}

fn specialist_artifact_id(run_id: &str, agent_key: &str) -> String {
    format!("specialist:{run_id}:{agent_key}")
}

fn section_artifact_id(run_id: &str, zero_based_idx: usize) -> String {
    format!("section:{run_id}:sec-{:02}", zero_based_idx + 1)
}

fn persist_provenance_edge(
    db: &DbInstance,
    run_id: &str,
    edge_index: usize,
    from: &str,
    to: &str,
    edge_type: &str,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let edge_id = format!("{run_id}:edge-{edge_index:04}");
    let mut params = BTreeMap::new();
    params.insert("eid".into(), cozo::DataValue::from(edge_id.as_str()));
    params.insert("from".into(), cozo::DataValue::from(from));
    params.insert("to".into(), cozo::DataValue::from(to));
    params.insert("typ".into(), cozo::DataValue::from(edge_type));
    params.insert("ca".into(), cozo::DataValue::from(now.as_str()));

    db.run_script(
        "?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at] \
         <- [[$eid, $from, $to, $typ, $ca]] \
         :put gt_provenance_edges { edge_id => from_artifact_id, to_artifact_id, edge_type, created_at }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("persist gt_provenance_edges failed: {e}"))?;
    Ok(())
}

pub(super) fn persist_run_checkpoint(
    db: &DbInstance,
    run_id: &str,
    checkpoint_key: &str,
    checkpoint_type: &str,
    status: &str,
    detail: serde_json::Value,
) -> Result<()> {
    ensure_gametheory_schema(db)?;
    let detail_json = serde_json::to_string(&detail)
        .map_err(|e| anyhow::anyhow!("serialize checkpoint detail failed: {e}"))?;
    let created_at = Utc::now().to_rfc3339();

    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("ck".into(), cozo::DataValue::from(checkpoint_key));
    params.insert("ct".into(), cozo::DataValue::from(checkpoint_type));
    params.insert("st".into(), cozo::DataValue::from(status));
    params.insert("dj".into(), cozo::DataValue::from(detail_json.as_str()));
    params.insert("ca".into(), cozo::DataValue::from(created_at.as_str()));

    db.run_script(
        "?[run_id, checkpoint_key, checkpoint_type, status, detail_json, created_at] \
         <- [[$rid, $ck, $ct, $st, $dj, $ca]] \
         :put gt_run_checkpoints { run_id, checkpoint_key => checkpoint_type, status, detail_json, created_at }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("persist gt_run_checkpoints failed: {e}"))?;
    Ok(())
}

pub(super) fn persist_tier_checkpoints(
    db: &DbInstance,
    run_id: &str,
    outputs: &HashMap<String, String>,
) -> Result<()> {
    let mut by_tier: BTreeMap<u8, Vec<&str>> = BTreeMap::new();
    for agent_key in outputs.keys() {
        if let Some(tier) = agent_tier(agent_key) {
            by_tier.entry(tier).or_default().push(agent_key.as_str());
        }
    }

    for (tier, agents) in by_tier {
        persist_run_checkpoint(
            db,
            run_id,
            &format!("tier:{tier}"),
            "tier",
            "completed",
            serde_json::json!({"tier": tier, "completed_agents": agents}),
        )?;
    }
    Ok(())
}

pub(super) fn insert_gt_run(
    db: &DbInstance,
    run_id: &str,
    situation: &str,
    started_at: &str,
    status: &str,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("sit".into(), cozo::DataValue::from(situation));
    params.insert("sa".into(), cozo::DataValue::from(started_at));
    params.insert("st".into(), cozo::DataValue::from(status));

    db.run_script(
        "?[run_id, situation, started_at, completed_at, status, cost_usd] \
         <- [[$rid, $sit, $sa, \"\", $st, \"0.000000\"]] \
         :put gt_runs { run_id => situation, started_at, completed_at, status, cost_usd }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert gt_runs failed: {e}"))?;
    Ok(())
}

pub(super) fn update_gt_run_status(
    db: &DbInstance,
    run_id: &str,
    situation: &str,
    started_at: &str,
    completed_at: &str,
    status: &str,
    cost_usd: f64,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("sit".into(), cozo::DataValue::from(situation));
    params.insert("sa".into(), cozo::DataValue::from(started_at));
    params.insert("ca".into(), cozo::DataValue::from(completed_at));
    params.insert("st".into(), cozo::DataValue::from(status));
    let cost = format!("{cost_usd:.6}");
    params.insert("cost".into(), cozo::DataValue::from(cost.as_str()));

    db.run_script(
        "?[run_id, situation, started_at, completed_at, status, cost_usd] \
         <- [[$rid, $sit, $sa, $ca, $st, $cost]] \
         :put gt_runs { run_id => situation, started_at, completed_at, status, cost_usd }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("update gt_runs failed: {e}"))?;
    Ok(())
}

pub(super) fn insert_gt_fingerprint(
    db: &DbInstance,
    run_id: &str,
    fingerprint_json: &str,
    primary_family: &str,
    created_at: &str,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    params.insert("fp".into(), cozo::DataValue::from(fingerprint_json));
    params.insert("pf".into(), cozo::DataValue::from(primary_family));
    params.insert("ca".into(), cozo::DataValue::from(created_at));

    db.run_script(
        "?[run_id, fingerprint_json, primary_family, created_at] \
         <- [[$rid, $fp, $pf, $ca]] \
         :put gt_fingerprints { run_id => fingerprint_json, primary_family, created_at }",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert gt_fingerprints failed: {e}"))?;
    Ok(())
}
