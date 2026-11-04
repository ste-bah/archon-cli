//! Socket-free checks use the same extractors and validators as the host gates.
use crate::server::variants::r17::{self, Cell};
use archon_workflow::{obligation_ids::*, task_skeleton::*};
use serde_json::Value;

const PRD: &str = include_str!("../fixtures/offline_decomposition_r17/prd.md");

#[test]
fn r17_pin_selection_excludes_locks_and_history_directories() {
    let temp = tempfile::tempdir().unwrap();
    let pins = temp.path().join(".archon/task-set-pins");
    std::fs::create_dir_all(pins.join("predecessors.json")).unwrap();
    std::fs::write(pins.join("set.chain.lock"), "").unwrap();
    std::fs::write(pins.join("set.publish.lock"), "").unwrap();
    std::fs::write(pins.join("set.json"), r#"{"skeleton_digest":"frozen"}"#).unwrap();
    let pin = crate::assertions::pin_path(temp.path());
    assert_eq!(pin, pins.join("set.json"));
    assert_eq!(crate::assertions::read(&pin)["skeleton_digest"], "frozen");
}

#[test]
fn r17_v4_truncation_stalls_once_then_allows_a_fresh_gate() {
    use crate::server::RequestRecord;
    use serde_json::json;
    let record = |answer: &str| RequestRecord {
        method: "POST".into(),
        path: "/v1/messages".into(),
        auth_present: true,
        known_path: true,
        role: "fidelity".into(),
        key: "fidelity".into(),
        prompt: String::new(),
        answer: answer.into(),
    };
    let prompt = json!({"messages":[{"role":"user", "content":
        "===== BEGIN TASK TASK-X-010 =====\n===== BEGIN TASK TASK-X-020 ====="}]});
    let good = r#"{"verdicts":[]}"#;
    let mut previous = vec![record(good), record(good)];
    let head = r17::critic(&prompt.to_string(), good.into(), Cell::V4, &previous);
    assert_eq!(head, "{\"verdicts\":[");
    previous.push(record(&head));
    let mut continuation = prompt.clone();
    continuation["messages"].as_array_mut().unwrap().extend([
        json!({"role":"assistant", "content":head}),
        json!({"role":"user", "content":"Continue from the next character."}),
    ]);
    assert_eq!(
        r17::critic(&continuation.to_string(), good.into(), Cell::V4, &previous),
        ""
    );
    previous.push(record(""));
    assert_eq!(
        r17::critic(&prompt.to_string(), good.into(), Cell::V4, &previous),
        good
    );
}

fn prompt(attempt: usize) -> String {
    let marker = include_str!("../../src/command/workflow_decompose_v1_frozen.js")
        .split_once("const SKELETON_AUTHOR_MARKER = \"")
        .unwrap()
        .1
        .split('"')
        .next()
        .unwrap();
    format!("{marker}\nRepair operation r1, cycle 1, authored call {attempt}.")
}

#[test]
fn r17_fixture_shapes_are_visible_to_the_production_extractors() {
    assert_eq!(
        obligation_ids(PRD),
        ["REQ-X-001", "REQ-X-002", "REQ-X-003", "AC-X-001"]
            .map(str::to_owned)
            .into()
    );
    assert_eq!(
        constraint_ids(PRD),
        ["C-X-001", "C-X-002", "C-X-003"].map(str::to_owned).into()
    );
    let sections = numbered_sections(PRD);
    assert!(sections["0"].requires_citation);
    assert!(!sections["3"].requires_citation);
    assert!(sections["3.1"].requires_citation && sections["3.2"].requires_citation);
    assert!(!sections.contains_key("7"));
    for key in ["1", "2", "4", "5", "6", "8", "9"] {
        assert!(sections[key].requires_citation);
    }
    let candidate: TaskSkeleton =
        serde_json::from_value(r17::skeleton(&prompt(1), Cell::V0)).unwrap();
    validate_skeleton(&candidate, "").unwrap();
    assert!(constraint_owner_findings(&candidate, &constraint_texts(PRD)).is_empty());
    assert!(
        section_citation_findings(
            PRD,
            candidate
                .tasks
                .iter()
                .map(|t| (t.task_id.as_str(), t.source_sections.as_slice()))
        )
        .is_empty()
    );
}

#[test]
fn r17_v1_validator_fault_set_and_repaired_candidate_are_exact() {
    let candidate: TaskSkeleton =
        serde_json::from_value(r17::skeleton(&prompt(1), Cell::V1)).unwrap();
    let constraints = constraint_ids(PRD);
    let mut allowed = obligation_ids(PRD);
    allowed.extend(constraints.iter().cloned());
    let mut codes: Vec<_> = validate_skeleton_set(&candidate, &allowed)
        .into_iter()
        // The freeze adapter routes constraint ownership through the shared
        // constraint rule, rather than also calling it an unowned obligation.
        .filter(|f| {
            !(f.identity.code == "unowned_obligation" && constraints.contains(&f.identity.subject))
        })
        .map(|f| f.identity.code)
        .collect();
    let owned = constraint_owner_findings(&candidate, &constraint_texts(PRD));
    assert_eq!(owned.len(), 1);
    assert_eq!(owned[0].identity.subject, "C-X-002");
    codes.extend(owned.into_iter().map(|f| f.identity.code));
    let cited = section_citation_findings(
        PRD,
        candidate
            .tasks
            .iter()
            .map(|t| (t.task_id.as_str(), t.source_sections.as_slice())),
    );
    assert_eq!(cited.len(), 1);
    assert_eq!(cited[0].identity.subject, "9");
    codes.extend(cited.into_iter().map(|f| f.identity.code));
    codes.sort();
    assert_eq!(
        codes,
        ["duplicate_entry", "uncited_section", "unowned_constraint"]
    );
    let fixed: TaskSkeleton = serde_json::from_value(r17::skeleton(&prompt(2), Cell::V1)).unwrap();
    assert!(skeleton_defects(&fixed, "").is_empty());
    assert!(constraint_owner_findings(&fixed, &constraint_texts(PRD)).is_empty());
    assert!(
        section_citation_findings(
            PRD,
            fixed
                .tasks
                .iter()
                .map(|t| (t.task_id.as_str(), t.source_sections.as_slice()))
        )
        .is_empty()
    );
}

#[test]
fn r17_v2_body_fault_is_yaml_only_then_names_a_file() {
    for (attempt, must_name) in [(1, false), (2, true)] {
        let (_, body) = r17::answer(
            &format!(
                "Author the complete TASK body for host-frozen task_id TASK-X-010. Repair operation r1, cycle 1, authored call {attempt}."
            ),
            Cell::V2,
        );
        let (yaml, prose) = body.split_once("\n```\n").unwrap();
        let names_constraint = |text: &str| {
            text.split(|c: char| !c.is_alphanumeric() && c != '-')
                .any(|token| token == "C-X-001")
        };
        assert_eq!(names_constraint(yaml), !must_name);
        assert_eq!(names_constraint(prose), must_name, "{body}");
        if must_name {
            assert!(
                prose
                    .lines()
                    .any(|line| line.contains("C-X-001") && line.contains("src/alpha.txt"))
            );
        }
    }
}

#[test]
fn r17_v3_binding_fields_survive_frozen_task_serde() {
    let candidate = r17::skeleton(&prompt(1), Cell::V3);
    let parsed: TaskSkeleton = serde_json::from_value(candidate).unwrap();
    let task: Value = serde_json::to_value(&parsed.tasks[0]).unwrap();
    assert_eq!(
        task["deliverable_contracts"][0]["instance_source_records_field"],
        "items"
    );
    assert_eq!(
        task["deliverable_contracts"][0]["registry_records_field"],
        "entries"
    );
    let edge = &parsed.tasks[1].depends_on[0];
    assert!(!edge.ordering_only);
    assert_eq!(
        edge.consumes[0].instance_source_records_field.as_deref(),
        Some("items")
    );
    assert_eq!(
        edge.consumes[0].registry_records_field.as_deref(),
        Some("entries")
    );
}
