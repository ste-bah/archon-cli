use super::*;

#[test]
fn required_tools_and_none_sections_reach_offline_skeleton_freeze_with_citations() {
    for declaration in [
        "None.",
        "| Tool | Used by | Purpose |\n| --- | --- | --- |\n| mcp__sample__fetch | AC-X-001 | work |",
        "| Tool | Used by | Purpose |\n| --- | --- | --- |\n| mcp__sample__inspect | AC-X-001 | work |",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let tasks = temp.path().join("tasks/PRD-X");
        let prd = temp.path().join("prds/PRD-X.md");
        std::fs::create_dir_all(&tasks).unwrap();
        std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
        let text = format!(
            "## 0. Acceptance Criteria\n| ID | Criterion |\n| --- | --- |\n| AC-X-001 | output is valid |\n\n## 1. Required Tools\n{declaration}\n"
        );
        std::fs::write(&prd, &text).unwrap();
        std::fs::write(temp.path().join(".mcp.json"), r#"{"mcpServers":{"sample":{"toolPolicy":{"toolPermissions":{"fetch":"safe","inspect":"safe"}}}}}"#).unwrap();
        let contract = serde_json::json!({
            "schema_version":1, "prd":{"path":"prds/PRD-X.md","digest":content_digest(text.as_bytes())},
            "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
            "acceptance":[{"id":"AC-X-001","criterion":"output is valid","check":{"kind":"command","command":"unused","cwd":"project_root"},"gap_permitted":false,"judgment":{"verdict":"accepted","counterexample":"attempted false state","reason":"observed","host_call_id":"test"}}],
            "supplementary":[]
        });
        let bytes = serde_json::to_vec(&contract).unwrap();
        let (lock, pin) = findings::acceptance_lock_and_pin(
            &tasks,
            archon_workflow::task_set_contract::FreezeGateMode::Enforce,
            &[],
            &bytes,
            None.into(),
        );
        std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), &bytes).unwrap();
        std::fs::write(
            tasks.join(ACCEPTANCE_LOCK_FILE),
            serde_json::to_vec(&lock).unwrap(),
        )
        .unwrap();
        let pin_path = acceptance_pin_path(temp.path(), &tasks);
        std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
        std::fs::write(pin_path, serde_json::to_vec(&pin).unwrap()).unwrap();
        let skeleton = serde_json::json!({"schema_version":1,"acceptance_digest":"pending","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010.md","implements":["AC-X-001"],"source_sections":["0","1"]}]});
        std::fs::write(
            tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
            serde_json::to_vec(&skeleton).unwrap(),
        )
        .unwrap();
        let result = freeze_skeleton(temp.path(), &tasks, &prd).unwrap();
        let frozen: archon_workflow::task_skeleton::TaskSkeletonLock = serde_json::from_slice(
            &std::fs::read(tasks.join(archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(frozen.digest, result.skeleton_digest);
        assert!(
            archon_workflow::obligation_ids::section_citation_findings(
                &text,
                [("TASK-X-010", ["0".to_owned(), "1".to_owned()].as_slice())]
            )
            .is_empty()
        );
    }
}
