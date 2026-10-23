use super::*;
use serde_json::json;

#[test]
fn post_heal_refresh_replaces_the_carried_candidate_before_script_planning() {
    let temp = tempfile::tempdir().unwrap();
    let published = entry("AC-X-001", "healed check");
    let contract = json!({
        "schema_version":1,"prd":{"path":"PRD.md","digest":"p"},
        "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
        "acceptance":[published],"supplementary":[]
    });
    let contract_bytes = serde_json::to_vec(&contract).unwrap();
    std::fs::write(
        temp.path()
            .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        &contract_bytes,
    )
    .unwrap();
    let mut seed = PhaseSeed {
        schema_version: 1,
        transition_index: 0,
        runtime: archon_workflow::FixedRunIdentityV1 {
            template_version: "t".into(),
            starting_binary_revision: "b".into(),
            script_digest: "s".into(),
            catalog_digest: "c".into(),
            project_root_identity: "p".into(),
            prd_identity: "p".into(),
            task_root_identity: "t".into(),
        },
        derived_at: "now".into(),
        author_ordinals: BTreeMap::new(),
        pause_ordinals: BTreeMap::new(),
        subjects: BTreeMap::from([(
            "acceptance".into(),
            SubjectSeed::Entries {
                gates: Vec::new(),
                candidate: None,
                replies: Vec::new(),
                unreadable_replies: BTreeMap::new(),
                carried_entries: vec![entry("AC-X-001", "pre-heal check")],
                refuted_ids: Vec::new(),
                invalid: BTreeMap::new(),
                carried: 1,
            },
        )]),
        acceptance_refreshed_from_contract_digest: None,
        resume_frozen_chain: None,
        pause_generation_floor: 0,
        history_attempts: BTreeMap::new(),
        event_id: None,
    };
    let mut arguments =
        crate::command::workflow_decompose_seed::seeded_arguments(&json!({}), Some(&seed));
    super::super::refresh_after_resume_heal(
        temp.path(),
        temp.path(),
        &mut arguments,
        Some(&mut seed),
        &json!({}),
    )
    .unwrap();
    assert_eq!(
        arguments["phaseSeed"]["subjects"]["acceptance"]["carried_entries"][0]["check"]["command"],
        "healed check"
    );
    assert_eq!(
        seed.acceptance_refreshed_from_contract_digest.as_deref(),
        Some(archon_workflow::task_set_contract::content_digest(&contract_bytes).as_str())
    );
}
