use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use archon_cognitive::self_model::SelfModelStore;
use archon_cognitive::{
    CognitiveInspection, CognitiveInspectionStatus, CognitiveTick, DecisionRecord,
    PersistentCognitiveStore, ProposalSummary, ReflectionSummary, TickReport,
};

use crate::cli_args::CognitiveAction;

pub(crate) async fn handle_cognitive_command(action: &CognitiveAction) -> Result<()> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    match action {
        CognitiveAction::Status { json } => {
            let bundle = open_bundle(&cwd)?;
            let status = bundle.inspection()?.status()?;
            print_status(&status, *json)
        }
        CognitiveAction::Tick { json } => {
            let bundle = open_bundle(&cwd)?;
            let policy = archon_policy::load_effective_policy(&cwd)
                .map(|policy| policy.cognitive)
                .unwrap_or_default();
            let report = CognitiveTick::new(bundle.store.db(), Some(policy))?.tick()?;
            print_tick(&report, *json)
        }
        CognitiveAction::Inspect {
            decision_id,
            session,
            limit,
            json,
        } => {
            let bundle = open_bundle(&cwd)?;
            print_inspection(&bundle.inspection()?, decision_id, session, *limit, *json)
        }
        CognitiveAction::SelfModel { domains, json } => {
            let bundle = open_bundle(&cwd)?;
            let domains = if domains.is_empty() {
                vec![
                    "code_change".into(),
                    "research".into(),
                    "pipeline_control".into(),
                ]
            } else {
                domains.clone()
            };
            let profile = SelfModelStore::new(bundle.store.db())?.read_profile(&domains)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&profile)?);
            } else {
                println!("Cognitive self-model");
                println!("Generated: {}", profile.generated_at);
                for trust in profile.domain_trust {
                    println!(
                        "- {} trust={:.2} evidence={} failures={}",
                        trust.domain,
                        trust.trust_score,
                        trust.evidence_count,
                        trust.failure_cluster_ids.len()
                    );
                }
                if !profile.caution_rules.is_empty() {
                    println!("Caution rules:");
                    for rule in profile.caution_rules {
                        println!("- {rule}");
                    }
                }
            }
            Ok(())
        }
        CognitiveAction::Reflections {
            session,
            limit,
            json,
        } => {
            let bundle = open_bundle(&cwd)?;
            let reflections = bundle
                .inspection()?
                .reflections(session.as_deref(), *limit)?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&reflections)?);
            } else {
                print_reflections(&reflections);
            }
            Ok(())
        }
    }
}

struct CognitiveBundle {
    store: PersistentCognitiveStore,
    ledger_dir: PathBuf,
}

impl CognitiveBundle {
    fn inspection(&self) -> Result<CognitiveInspection<'_>> {
        CognitiveInspection::new(self.store.db(), &self.ledger_dir).map_err(Into::into)
    }
}

fn open_bundle(cwd: &Path) -> Result<CognitiveBundle> {
    let root = cwd.join(".archon").join("cognitive");
    let store = PersistentCognitiveStore::open(&root)
        .with_context(|| format!("open cognitive store at {}", root.display()))?;
    Ok(CognitiveBundle {
        store,
        ledger_dir: root,
    })
}

fn print_status(status: &CognitiveInspectionStatus, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(status)?);
        return Ok(());
    }
    println!("Cognitive executive loop");
    println!("Situations: {}", status.situation_count);
    println!("Tool decisions: {}", status.tool_decision_count);
    println!("Executive decisions: {}", status.executive_decision_count);
    println!("Reflections: {}", status.reflection_count);
    println!("Governed proposals: {}", status.proposal_count);
    println!("Apply results: {}", status.apply_result_count);
    println!("Self-model facts: {}", status.self_model_fact_count);
    if let Some(tick) = &status.latest_tick {
        println!(
            "Latest tick: {} proposals={} applied={} denied={} errors={}",
            tick.created_at,
            tick.proposals_evaluated,
            tick.proposals_auto_applied,
            tick.proposals_denied,
            tick.error_count
        );
    }
    print_proposals(&status.pending_proposals);
    Ok(())
}

fn print_tick(report: &TickReport, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }
    println!("Cognitive tick {}", report.tick_id);
    println!("Dead letters replayed: {}", report.dead_letters_replayed);
    println!("Proposals evaluated: {}", report.proposals_evaluated);
    println!("Proposals generated: {}", report.proposals_generated);
    println!("Auto-applied: {}", report.proposals_auto_applied);
    println!("Denied: {}", report.proposals_denied);
    println!("Self-model updated: {}", report.self_model_updated);
    println!("Duration: {} ms", report.duration_ms);
    if !report.errors.is_empty() {
        println!("Errors:");
        for error in &report.errors {
            println!("- {error}");
        }
    }
    Ok(())
}

fn print_inspection(
    inspection: &CognitiveInspection<'_>,
    decision_id: &Option<String>,
    session: &Option<String>,
    limit: usize,
    json: bool,
) -> Result<()> {
    if let Some(decision_id) = decision_id {
        let decision = inspection.inspect_decision(decision_id)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&decision)?);
        } else if let Some(decision) = decision {
            print_decision(&decision);
        } else {
            println!("No cognitive decision found for {decision_id}");
        }
        return Ok(());
    }
    if let Some(session) = session {
        let decisions = inspection.decisions_for_session(session, limit)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&decisions)?);
        } else {
            for decision in decisions {
                print_decision(&decision);
            }
        }
        return Ok(());
    }
    anyhow::bail!("provide a decision id or --session <session-id>")
}

fn print_decision(decision: &DecisionRecord) {
    println!("Decision {}", decision.decision_id);
    println!(
        "Session: {} turn {}",
        decision.session_id, decision.turn_number
    );
    println!("Selected candidate: {}", decision.selected_candidate_id);
    println!(
        "Rejected alternatives: {}",
        decision.rejected_alternatives.len()
    );
    println!("Summary: {}", decision.user_visible_summary);
    if let Some(policy) = &decision.policy_verdict {
        println!("Policy: {policy}");
    }
    if let Some(contract) = &decision.verification_contract {
        println!("Verification: {contract}");
    }
    println!("Created: {}", decision.created_at);
}

fn print_reflections(reflections: &[ReflectionSummary]) {
    if reflections.is_empty() {
        println!("No cognitive reflections found.");
        return;
    }
    println!("Cognitive reflections");
    for reflection in reflections {
        println!(
            "- {} session={} turn={} outcome={} propose={} lesson={}",
            reflection.reflection_id,
            reflection.session_id,
            reflection.turn_number,
            reflection.outcome,
            reflection.should_propose,
            reflection.lesson
        );
    }
}

fn print_proposals(proposals: &[ProposalSummary]) {
    if proposals.is_empty() {
        return;
    }
    println!("Pending/recent proposals:");
    for proposal in proposals {
        println!(
            "- {} {} risk={} evidence={} result={}",
            proposal.proposal_id,
            proposal.manifest_kind,
            proposal.risk_level,
            proposal.evidence_count,
            proposal.latest_result.as_deref().unwrap_or("pending")
        );
    }
}
