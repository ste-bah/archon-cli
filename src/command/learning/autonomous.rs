use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use cozo::DbInstance;

use archon_learning::models::PolicyDecision;

pub(crate) async fn run_learning_tick() -> Result<()> {
    let db_path = learning_db_path();
    let db = open_learning_db(&db_path)?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    let cwd = std::env::current_dir()?;
    let summary = run_tick_on_db(&db, &cwd)?;
    print_summary(&summary);
    Ok(())
}

fn learning_db_path() -> PathBuf {
    crate::command::store_paths::learning_db_path()
}

fn open_learning_db(path: &Path) -> Result<DbInstance> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    archon_learning::cozo_guard::open_sqlite_guarded(
        path.to_str().unwrap_or(""),
        "open autonomous learning db",
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

#[derive(Default)]
struct TickSummary {
    scanned_events: usize,
    generated: usize,
    inserted: usize,
    evaluated: usize,
    applied: usize,
    held: usize,
}

fn run_tick_on_db(db: &DbInstance, workspace_dir: &Path) -> Result<TickSummary> {
    let events =
        archon_learning::store::list_all_learning_events(db).map_err(|e| anyhow::anyhow!("{e}"))?;
    let proposals = archon_learning::proposal::generate_proposals_for_store(db, &events)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let inserted = persist_new_proposals(db, proposals.iter())?;
    let pending = archon_learning::store::list_behaviour_proposals(db, Some("Pending"))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let policy = archon_policy::load_effective_policy(workspace_dir)
        .map_err(|e| anyhow::anyhow!("failed to load policy: {e}"))?;
    let recent_incidents = recent_false_completion_count(&events);
    let (applied, held) = evaluate_pending(db, &policy, recent_incidents, &pending)?;

    Ok(TickSummary {
        scanned_events: events.len(),
        generated: proposals.len(),
        inserted,
        evaluated: pending.len(),
        applied,
        held,
    })
}

fn persist_new_proposals<'a>(
    db: &DbInstance,
    proposals: impl Iterator<Item = &'a archon_learning::models::BehaviourProposal>,
) -> Result<usize> {
    let existing = archon_learning::store::list_behaviour_proposals(db, None)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut inserted = 0;
    for proposal in proposals {
        if existing.iter().any(|prior| same_proposal(prior, proposal)) {
            continue;
        }
        archon_learning::store::insert_behaviour_proposal(db, proposal)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        inserted += 1;
    }
    Ok(inserted)
}

fn same_proposal(
    left: &archon_learning::models::BehaviourProposal,
    right: &archon_learning::models::BehaviourProposal,
) -> bool {
    left.manifest_kind == right.manifest_kind && left.source_evidence() == right.source_evidence()
}

trait ProposalEvidence {
    fn source_evidence(&self) -> BTreeSet<&str>;
}

impl ProposalEvidence for archon_learning::models::BehaviourProposal {
    fn source_evidence(&self) -> BTreeSet<&str> {
        self.evidence_ids.iter().map(String::as_str).collect()
    }
}

fn evaluate_pending(
    db: &DbInstance,
    policy: &archon_policy::EffectivePolicy,
    recent_incidents: usize,
    pending: &[archon_learning::models::BehaviourProposal],
) -> Result<(usize, usize)> {
    let mut applied = 0;
    let mut held = 0;
    for proposal in pending {
        let (decision, _) = archon_learning::policy::evaluate_proposal_with_policy(
            db,
            proposal,
            policy,
            recent_incidents,
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        if decision != PolicyDecision::AutoApplied {
            held += 1;
            continue;
        }
        archon_learning::apply::apply_decision(
            db,
            &proposal.proposal_id,
            decision,
            None,
            Some("learning-autonomous"),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        applied += 1;
    }
    Ok((applied, held))
}

fn recent_false_completion_count(events: &[archon_learning::models::LearningEvent]) -> usize {
    events
        .iter()
        .filter(|event| {
            event.event_type == archon_learning::models::LearningEventType::FalseCompletionDetected
        })
        .count()
}

fn print_summary(summary: &TickSummary) {
    println!("Autonomous learning tick");
    println!("========================");
    println!("Events scanned:     {}", summary.scanned_events);
    println!("Proposals generated: {}", summary.generated);
    println!("Proposals inserted:  {}", summary.inserted);
    println!("Pending evaluated:   {}", summary.evaluated);
    println!("Auto-applied:        {}", summary.applied);
    println!("Held by policy:      {}", summary.held);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-learning-tick-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    fn record_correction(db: &DbInstance, id: &str) {
        archon_learning::events::record_event(
            db,
            "workspace",
            archon_learning::models::LearningEventType::UserCorrected,
            "rule-autonomous",
            None,
            serde_json::json!({ "id": id }),
            1.0,
            "",
        )
        .unwrap();
    }

    #[test]
    fn tick_autonomously_applies_when_policy_allows_high_risk() {
        let db = test_db();
        record_correction(&db, "1");
        record_correction(&db, "2");
        record_correction(&db, "3");
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join(".archon")).unwrap();
        std::fs::write(
            workspace.path().join(".archon/policy.toml"),
            "[policy.learning]\n\
             autonomous_apply = true\n\
             autonomous_max_risk = \"High\"\n\
             autonomous_min_evidence = 3\n\
             require_approval_for_prompt_changes = false\n\
             require_approval_for_blocking_gates = false\n\
             require_approval_for_network_changes = false\n",
        )
        .unwrap();

        let summary = run_tick_on_db(&db, workspace.path()).unwrap();

        assert_eq!(summary.generated, 1);
        assert_eq!(summary.inserted, 1);
        assert_eq!(summary.applied, 1);
        let applied =
            archon_learning::store::list_behaviour_proposals(&db, Some("Applied")).unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(
            applied[0].status,
            archon_learning::models::ProposalStatus::Applied
        );
    }
}
