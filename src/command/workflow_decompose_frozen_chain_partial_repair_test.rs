use super::*;

#[tokio::test]
async fn a_partial_launch_repair_records_publication_then_routes_pending_checks() {
    use crate::command::workflow_task_set::reauthor::test_client::{
        ScriptedAuthorJudge, command_entry,
    };

    let prd = "# PRD X\n\n## Requirements\n\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Prove the fixture. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The first fixture is proven. |\n";
    let fixture = frozen_with_prd(Some(names_live_root), prd);
    let contract_path = fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(&contract_path).unwrap()).unwrap();
    let mut second = contract.acceptance[0].clone();
    second.id = "AC-X-002".into();
    second.criterion = "The second fixture is proven.".into();
    contract.acceptance.push(second);
    let prd_bytes = format!("{prd}| AC-X-002 | The second fixture is proven. |\n");
    std::fs::write(fixture.project.join("prds/PRD-X.md"), &prd_bytes).unwrap();
    contract.prd.digest = content_digest(prd_bytes.as_bytes());
    for entry in &mut contract.acceptance {
        entry.check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
            command: names_live_root(&fixture.project),
            cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
        };
    }
    let bytes = serde_json::to_vec_pretty(&contract).unwrap();
    let digest = content_digest(&bytes);
    std::fs::write(&contract_path, &bytes).unwrap();
    for path in [
        fixture.tasks.join(ACCEPTANCE_LOCK_FILE),
        crate::command::workflow_task_set::acceptance_pin_path(&fixture.project, &fixture.tasks),
    ] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let key = if value.get("digest").is_some() {
            "digest"
        } else {
            "acceptance_digest"
        };
        value[key] = digest.clone().into();
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    crate::command::workflow_task_set::passability_tests_support::commit_project(&fixture.project);

    let client = std::sync::Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f marker"),
        |id, _| id == "AC-X-001",
    ));
    let factory = ReauthorThenBarrier {
        reauthor: std::sync::Mutex::new(Some(client)),
        origins: std::sync::Mutex::new(Vec::new()),
    };
    let error = format!("{:#}", launch_with(&fixture, &factory).await);
    assert!(error.contains("barrier observed"), "{error}");

    let log = std::fs::read_to_string(fixture.tasks.join(".decompose.log")).unwrap();
    assert!(
        log.contains(
            "frozen_chain_launch acceptance=reauthored published_ids=AC-X-001 pending_ids=AC-X-002"
        ),
        "{log}"
    );
    assert!(
        log.contains("frozen_chain_launch acceptance=reauthor_refuted_routed ids=AC-X-002 detail="),
        "{log}"
    );

    let run = fixture.store.list_runs().unwrap().pop().unwrap();
    let events = std::fs::read_to_string(fixture.store.events_path(&run.id)).unwrap();
    let repair_event = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| {
            event["detail"]["event"]
                == crate::command::workflow_decompose_launch_repair::REAUTHORED_EVENT
        })
        .unwrap_or_else(|| panic!("no repair event: {events}"));
    assert_eq!(
        repair_event["detail"]["published_ids"],
        serde_json::json!(["AC-X-001"])
    );
    assert_eq!(
        repair_event["detail"]["pending_ids"],
        serde_json::json!(["AC-X-002"])
    );
    let arguments: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            fixture
                .store
                .run_dir(&run.id)
                .join(crate::command::workflow_decompose::FIXED_ARGUMENTS_PATH),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(arguments["frozenChain"]["acceptance"], false, "{arguments}");

    let contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(contract_path).unwrap()).unwrap();
    assert!(contract.acceptance.iter().any(|entry| {
        entry.id == "AC-X-001"
            && matches!(
                &entry.check,
                archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. }
                    if command == "test -f marker"
            )
    }));
}
