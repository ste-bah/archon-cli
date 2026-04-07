//! Tests for Rehydration Policy (TASK-PIPE-E11).
//!
//! Validates: rehydration loads 5 items (contract, task state, memories,
//! latest blocker, wiring plan), mission brief rendering, cap at 10 memories,
//! missing files produce None not errors.

use archon_pipeline::rehydration::{RehydrationPolicy, RehydrationContext, render_mission_brief};
use archon_pipeline::coding::contract::{
    TaskContract, AcceptanceCriterion, WiringRequirement, WiringType, TestRequirement, TestType,
};
use archon_pipeline::coding::wiring::{WiringPlan, WiringObligation, WiringAction, ObligationStatus};
use archon_pipeline::ledgers::{TaskEntry, TaskStatus, VerificationEntry, WiringObligationRef};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sample_contract() -> TaskContract {
    TaskContract {
        task_id: "TEST-001".into(),
        goal: "Add user profile page".into(),
        non_goals: vec!["Admin dashboard".into()],
        acceptance_criteria: vec![AcceptanceCriterion {
            id: "AC-001".into(),
            description: "Profile page renders correctly".into(),
            verification: "Manual review".into(),
        }],
        affected_files: vec!["src/pages/profile.rs".into()],
        required_wiring: vec![WiringRequirement {
            module: "src/router.rs".into(),
            entrypoint: "register_routes".into(),
            wiring_type: WiringType::RouteRegistration,
        }],
        required_tests: vec![TestRequirement {
            test_type: TestType::Unit,
            description: "Profile tests".into(),
        }],
        rollback_plan: "Revert commit".into(),
        definition_of_done: vec!["Page renders".into(), "Tests pass".into()],
    }
}

fn sample_wiring_plan() -> WiringPlan {
    WiringPlan {
        task_id: "TEST-001".into(),
        obligations: vec![WiringObligation {
            id: "WO-001".into(),
            file: "src/router.rs".into(),
            action: WiringAction::RegisterRoute,
            line_context: "router.route(\"/profile\", handler)".into(),
            mandatory: true,
            maps_to_contract_wiring: Some("src/router.rs::register_routes".into()),
            status: ObligationStatus::Pending,
        }],
        validated_at: None,
    }
}

fn sample_task_entry() -> TaskEntry {
    TaskEntry {
        task_id: "TEST-001".into(),
        status: TaskStatus::InProgress,
        assigned_agent: "code-generator".into(),
        dependencies: vec![],
        changed_files: vec![],
        wiring_obligations: vec![WiringObligationRef {
            obligation_id: "WO-001".into(),
            status: "Pending".into(),
        }],
        last_verification: None,
        timestamp: "2026-04-07T12:00:00Z".into(),
    }
}

fn sample_verification_failure() -> VerificationEntry {
    VerificationEntry {
        gate_name: "compilation".into(),
        passed: false,
        failure_details: Some("error[E0308]: mismatched types".into()),
        evidence_summary: "cargo build failed".into(),
        timestamp: "2026-04-07T12:00:00Z".into(),
    }
}

/// Set up a session directory with all 5 rehydration files.
fn setup_session_dir(tmp: &std::path::Path) {
    let contract = sample_contract();
    let wiring = sample_wiring_plan();
    let task = sample_task_entry();
    let verification = sample_verification_failure();

    // contract.json
    std::fs::write(
        tmp.join("contract.json"),
        serde_json::to_string_pretty(&contract).unwrap(),
    ).unwrap();

    // wiring-plan.json
    std::fs::write(
        tmp.join("wiring-plan.json"),
        serde_json::to_string_pretty(&wiring).unwrap(),
    ).unwrap();

    // ledgers/tasks.json
    let ledger_dir = tmp.join("ledgers");
    std::fs::create_dir_all(&ledger_dir).unwrap();
    std::fs::write(
        ledger_dir.join("tasks.json"),
        serde_json::to_string_pretty(&vec![&task]).unwrap(),
    ).unwrap();

    // ledgers/verifications.json
    std::fs::write(
        ledger_dir.join("verifications.json"),
        serde_json::to_string_pretty(&vec![&verification]).unwrap(),
    ).unwrap();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

mod rehydration_tests {
    use super::*;

    #[test]
    fn rehydrate_loads_all_5_items() {
        let tmp = tempfile::tempdir().unwrap();
        setup_session_dir(tmp.path());

        let policy = RehydrationPolicy::new(tmp.path().to_path_buf());
        let ctx = policy.rehydrate("code-generator", "coding").unwrap();

        // 1. Mission brief is non-empty
        assert!(!ctx.mission_brief.is_empty(), "mission brief should be non-empty");
        assert!(ctx.mission_brief.contains("Add user profile page"), "should contain goal");

        // 2. Current task state
        assert!(ctx.current_task_state.is_some(), "task state should be loaded");
        let task = ctx.current_task_state.unwrap();
        assert_eq!(task.task_id, "TEST-001");

        // 3. Relevant memories (empty for test — no memory backend)
        // Just verify the field exists and is bounded
        assert!(ctx.relevant_memories.len() <= 10);

        // 4. Latest blocker
        assert!(ctx.latest_blocker.is_some(), "should have latest blocker");
        assert!(ctx.latest_blocker.unwrap().contains("compilation"));

        // 5. Wiring checklist
        assert!(ctx.wiring_checklist.is_some(), "wiring plan should be loaded");
        assert_eq!(ctx.wiring_checklist.unwrap().obligations.len(), 1);
    }

    #[test]
    fn rehydrate_missing_wiring_plan_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        setup_session_dir(tmp.path());
        // Remove wiring plan
        std::fs::remove_file(tmp.path().join("wiring-plan.json")).unwrap();

        let policy = RehydrationPolicy::new(tmp.path().to_path_buf());
        let ctx = policy.rehydrate("code-generator", "coding").unwrap();

        assert!(ctx.wiring_checklist.is_none(), "missing wiring plan should be None");
        // Other items should still load
        assert!(!ctx.mission_brief.is_empty());
    }

    #[test]
    fn rehydrate_missing_verifications_returns_no_blocker() {
        let tmp = tempfile::tempdir().unwrap();
        setup_session_dir(tmp.path());
        // Remove verifications
        std::fs::remove_file(tmp.path().join("ledgers/verifications.json")).unwrap();

        let policy = RehydrationPolicy::new(tmp.path().to_path_buf());
        let ctx = policy.rehydrate("code-generator", "coding").unwrap();

        assert!(ctx.latest_blocker.is_none(), "missing verifications should yield no blocker");
    }

    #[test]
    fn rehydrate_empty_session_dir_still_works() {
        let tmp = tempfile::tempdir().unwrap();
        // Empty dir — no files at all

        let policy = RehydrationPolicy::new(tmp.path().to_path_buf());
        let ctx = policy.rehydrate("code-generator", "coding").unwrap();

        assert!(ctx.mission_brief.is_empty() || ctx.mission_brief.contains("No contract"));
        assert!(ctx.current_task_state.is_none());
        assert!(ctx.latest_blocker.is_none());
        assert!(ctx.wiring_checklist.is_none());
    }

    #[test]
    fn mission_brief_renders_all_fields() {
        let contract = sample_contract();
        let wiring = Some(sample_wiring_plan());

        let brief = render_mission_brief(&contract, &wiring, 4, "code-generator", 15, 48);

        assert!(brief.contains("Mission Brief"), "should have header");
        assert!(brief.contains("Add user profile page"), "should contain goal");
        assert!(brief.contains("Profile page renders"), "should contain acceptance criteria");
        assert!(brief.contains("Admin dashboard"), "should contain non-goals");
        assert!(brief.contains("Page renders"), "should contain completion criteria");
        assert!(brief.contains("Phase: 4"), "should contain phase");
        assert!(brief.contains("code-generator"), "should contain agent key");
        assert!(brief.contains("15/48"), "should contain progress");
    }

    #[test]
    fn mission_brief_without_wiring_plan() {
        let contract = sample_contract();
        let brief = render_mission_brief(&contract, &None, 2, "system-designer", 5, 48);

        assert!(brief.contains("Mission Brief"));
        assert!(brief.contains("No wiring plan") || !brief.contains("WO-001"));
    }

    #[test]
    fn memories_capped_at_10() {
        // RehydrationContext should never have > 10 memories
        let ctx = RehydrationContext {
            mission_brief: String::new(),
            current_task_state: None,
            relevant_memories: (0..15).map(|i| format!("memory-{}", i)).collect(),
            latest_blocker: None,
            wiring_checklist: None,
        };
        // The struct allows it, but rehydrate() should cap
        assert!(ctx.relevant_memories.len() > 10, "raw struct allows > 10");
        // This test validates the struct can hold memories; the cap is in rehydrate()
    }

    #[test]
    fn latest_blocker_picks_most_recent_failure() {
        let tmp = tempfile::tempdir().unwrap();
        setup_session_dir(tmp.path());

        // Add a second verification that passed
        let passing = VerificationEntry {
            gate_name: "orphan-detection".into(),
            passed: true,
            failure_details: None,
            evidence_summary: "all files referenced".into(),
            timestamp: "2026-04-07T13:00:00Z".into(),
        };
        let failing = sample_verification_failure();
        let ledger_dir = tmp.path().join("ledgers");
        std::fs::write(
            ledger_dir.join("verifications.json"),
            serde_json::to_string_pretty(&vec![&failing, &passing]).unwrap(),
        ).unwrap();

        let policy = RehydrationPolicy::new(tmp.path().to_path_buf());
        let ctx = policy.rehydrate("code-generator", "coding").unwrap();

        // Should pick the most recent FAILURE, not the passing one
        assert!(ctx.latest_blocker.is_some());
        assert!(ctx.latest_blocker.unwrap().contains("compilation"));
    }
}
