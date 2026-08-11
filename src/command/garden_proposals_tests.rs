//! The review surface, driven end to end against a real store.
//!
//! Each test reads the memory store back rather than trusting the command's
//! output. A message saying "applied" is not evidence that anything was applied.

use std::sync::Arc;

use archon_learning::garden_proposals::{
    GardenProposalKind, GardenProposalRecord, GardenProposalStatus, get_garden_proposal,
    insert_garden_proposal,
};
use archon_memory::garden::SemanticConsolidationCandidate;
use archon_memory::types::{MemoryType, SearchFilter};
use archon_memory::{MemoryGraph, MemoryTrait};

use super::handle;
use crate::command::test_support::CtxBuilder;

fn learning_db() -> Arc<cozo::DbInstance> {
    let db = Arc::new(
        cozo::DbInstance::new("mem", "", Default::default()).expect("in-memory learning db"),
    );
    archon_learning::schema::ensure_learning_schema(&db).expect("schema");
    db
}

fn args(sub: &str, argument: &str) -> Vec<String> {
    if argument.is_empty() {
        vec![sub.to_string()]
    } else {
        vec![sub.to_string(), argument.to_string()]
    }
}

fn visible(memory: &Arc<dyn MemoryTrait>, memory_type: MemoryType) -> Vec<String> {
    memory
        .search_memories(&SearchFilter {
            memory_type: Some(memory_type),
            ..SearchFilter::default()
        })
        .expect("search")
        .into_iter()
        .map(|row| row.id)
        .collect()
}

fn pending_retirement(subject: &str) -> GardenProposalRecord {
    GardenProposalRecord {
        proposal_id: GardenProposalRecord::stable_id(GardenProposalKind::MemoryRetirement, subject),
        proposal_kind: GardenProposalKind::MemoryRetirement,
        subject_id: subject.to_string(),
        subject_title: "a title".to_string(),
        excerpt: "an excerpt".to_string(),
        detail: "untouched for 91 days".to_string(),
        payload_json: "{}".to_string(),
        run_id: "run-1".to_string(),
        status: GardenProposalStatus::Pending,
        applied_ref: String::new(),
        created_at: "2026-08-10T03:00:00Z".to_string(),
        decided_at: String::new(),
    }
}

/// A store, a governed db, and a context wired to both.
struct Fixture {
    db: Arc<cozo::DbInstance>,
    memory: Arc<dyn MemoryTrait>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let graph = MemoryGraph::in_memory().expect("graph");
        Self {
            db: learning_db(),
            memory: Arc::new(graph) as Arc<dyn MemoryTrait>,
            _dir: tempfile::tempdir().expect("tempdir"),
        }
    }

    fn run(&self, sub: &str, argument: &str) -> bool {
        let (mut ctx, _rx) = CtxBuilder::new()
            .with_memory(Arc::clone(&self.memory))
            .with_governed_learning_db(Arc::clone(&self.db))
            .with_working_dir(self._dir.path().to_path_buf())
            .build();
        handle(&mut ctx, sub, &args(sub, argument))
    }

    fn status(&self, proposal_id: &str) -> GardenProposalStatus {
        get_garden_proposal(&self.db, proposal_id)
            .expect("get")
            .expect("present")
            .status
    }
}

#[test]
fn unrelated_subcommands_fall_through() {
    let fixture = Fixture::new();

    assert!(!fixture.run("stats", ""));
    assert!(!fixture.run("", ""));
    assert!(fixture.run("proposals", ""));
}

#[test]
fn approving_and_applying_retires_the_memory_and_rollback_restores_it() {
    // The whole loop, checked against the store at each step.
    let fixture = Fixture::new();
    let id = fixture
        .memory
        .store_memory(
            "a forgotten fact",
            "t",
            MemoryType::Fact,
            0.1,
            &[],
            "test",
            "",
        )
        .expect("store");
    let record = pending_retirement(&id);
    insert_garden_proposal(&fixture.db, &record).expect("insert");

    fixture.run("approve", &record.proposal_id);
    assert_eq!(
        fixture.status(&record.proposal_id),
        GardenProposalStatus::Approved
    );
    assert!(
        visible(&fixture.memory, MemoryType::Fact).contains(&id),
        "approval alone must not change the store"
    );

    fixture.run("apply", "");
    assert_eq!(
        fixture.status(&record.proposal_id),
        GardenProposalStatus::Applied
    );
    assert!(
        !visible(&fixture.memory, MemoryType::Fact).contains(&id),
        "the applied retirement must actually hide the memory"
    );
    assert!(
        fixture.memory.inspect_memory(&id).is_ok(),
        "and must not destroy it"
    );

    fixture.run("rollback", &record.proposal_id);
    assert_eq!(
        fixture.status(&record.proposal_id),
        GardenProposalStatus::RolledBack
    );
    assert!(
        visible(&fixture.memory, MemoryType::Fact).contains(&id),
        "rollback must bring the memory back to ordinary reads"
    );
}

#[test]
fn rejecting_leaves_the_memory_alone_and_apply_skips_it() {
    let fixture = Fixture::new();
    let id = fixture
        .memory
        .store_memory("a fact", "t", MemoryType::Fact, 0.1, &[], "test", "")
        .expect("store");
    let record = pending_retirement(&id);
    insert_garden_proposal(&fixture.db, &record).expect("insert");

    fixture.run("reject", &record.proposal_id);
    fixture.run("apply", "");

    assert_eq!(
        fixture.status(&record.proposal_id),
        GardenProposalStatus::Rejected
    );
    assert!(visible(&fixture.memory, MemoryType::Fact).contains(&id));
}

#[test]
fn apply_never_touches_a_proposal_nobody_approved() {
    // There must be no path from raised to applied without a decision.
    let fixture = Fixture::new();
    let id = fixture
        .memory
        .store_memory("a fact", "t", MemoryType::Fact, 0.1, &[], "test", "")
        .expect("store");
    let record = pending_retirement(&id);
    insert_garden_proposal(&fixture.db, &record).expect("insert");

    fixture.run("apply", "");

    assert_eq!(
        fixture.status(&record.proposal_id),
        GardenProposalStatus::Pending
    );
    assert!(
        visible(&fixture.memory, MemoryType::Fact).contains(&id),
        "an unapproved proposal changed the store"
    );
}

#[test]
fn applying_a_consolidation_writes_the_reviewed_text_and_rollback_withdraws_it() {
    let fixture = Fixture::new();
    let source = fixture
        .memory
        .store_memory(
            "always run the formatter before committing",
            "formatting",
            MemoryType::Fact,
            0.5,
            &[],
            "extraction",
            "",
        )
        .expect("store");
    let candidate = SemanticConsolidationCandidate {
        candidate_id: "cand-1".into(),
        proposed_content: "always run the formatter before committing".into(),
        proposed_title: "formatting".into(),
        memory_type: MemoryType::Fact,
        project_path: String::new(),
        source_type: "extraction".into(),
        proposed_importance: 0.65,
        representative_id: source.clone(),
        sources: vec![archon_memory::garden::ConsolidationSource {
            memory_id: source.clone(),
            excerpt: "always run the formatter".into(),
            importance: 0.5,
            created_at: chrono::Utc::now(),
        }],
    };
    let record = GardenProposalRecord {
        proposal_id: GardenProposalRecord::stable_id(
            GardenProposalKind::SemanticConsolidation,
            "cand-1",
        ),
        proposal_kind: GardenProposalKind::SemanticConsolidation,
        subject_id: "cand-1".to_string(),
        subject_title: "formatting".to_string(),
        excerpt: candidate.proposed_content.clone(),
        detail: "1 memory restates this".to_string(),
        payload_json: serde_json::to_string(&candidate).expect("payload"),
        run_id: "run-1".to_string(),
        status: GardenProposalStatus::Pending,
        applied_ref: String::new(),
        created_at: "2026-08-10T03:00:00Z".to_string(),
        decided_at: String::new(),
    };
    insert_garden_proposal(&fixture.db, &record).expect("insert");

    fixture.run("approve", &record.proposal_id);
    fixture.run("apply", "");

    let stored = get_garden_proposal(&fixture.db, &record.proposal_id)
        .expect("get")
        .expect("present");
    assert_eq!(stored.status, GardenProposalStatus::Applied);
    let derived = fixture
        .memory
        .inspect_memory(&stored.applied_ref)
        .expect("the consolidated memory exists");
    assert_eq!(
        derived.content, candidate.proposed_content,
        "the applied memory must carry exactly the reviewed text"
    );

    fixture.run("rollback", &record.proposal_id);
    assert!(
        !visible(&fixture.memory, MemoryType::Fact).contains(&stored.applied_ref),
        "the consolidated memory must be withdrawn"
    );
    assert!(
        visible(&fixture.memory, MemoryType::Fact).contains(&source),
        "rolling back a consolidation must not disturb its sources"
    );
}

#[test]
fn applying_a_rule_retirement_takes_it_out_of_the_prompt_and_rollback_restores_the_score() {
    let fixture = Fixture::new();
    let rule_id = fixture
        .memory
        .store_memory(
            "check constraints before acting",
            "",
            MemoryType::Rule,
            64.0,
            &["source:correction_derived".to_string()],
            "rules_engine",
            "",
        )
        .expect("store rule");
    let record = GardenProposalRecord {
        proposal_id: GardenProposalRecord::stable_id(GardenProposalKind::RuleRetirement, &rule_id),
        proposal_kind: GardenProposalKind::RuleRetirement,
        subject_id: rule_id.clone(),
        ..pending_retirement(&rule_id)
    };
    insert_garden_proposal(&fixture.db, &record).expect("insert");

    fixture.run("approve", &record.proposal_id);
    fixture.run("apply", "");
    assert!(
        !visible(&fixture.memory, MemoryType::Rule).contains(&rule_id),
        "a retired rule must leave the prompt block"
    );

    fixture.run("rollback", &record.proposal_id);
    let restored = fixture.memory.inspect_memory(&rule_id).expect("read");
    assert!(visible(&fixture.memory, MemoryType::Rule).contains(&rule_id));
    assert_eq!(
        restored.importance, 64.0,
        "the rule must return with the score its corrections earned"
    );
}

#[test]
fn a_rollback_id_that_names_nothing_is_reported_not_ignored() {
    let fixture = Fixture::new();

    assert!(fixture.run("rollback", "gp-nope"));
}

#[test]
fn decide_without_an_id_does_not_act_on_anything() {
    let fixture = Fixture::new();
    let id = fixture
        .memory
        .store_memory("a fact", "t", MemoryType::Fact, 0.1, &[], "test", "")
        .expect("store");
    insert_garden_proposal(&fixture.db, &pending_retirement(&id)).expect("insert");

    fixture.run("approve", "");

    assert_eq!(
        fixture.status(&GardenProposalRecord::stable_id(
            GardenProposalKind::MemoryRetirement,
            &id
        )),
        GardenProposalStatus::Pending
    );
}
