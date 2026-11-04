use super::*;

#[test]
#[allow(non_snake_case)]
fn offline_V3_tool_landing_then_freeze_skeleton() {
    use archon_workflow::task_set_contract::TASK_SKELETON_FILE;
    use archon_workflow::v2::record_landing::{RecordKind, RecordLanding};
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _) = seed_frozen_acceptance(&temp);
    let records = std::sync::Arc::new(
        RecordLanding::open(
            temp.path().join("records"),
            "fixture".into(),
            RecordKind::Skeleton,
            vec![],
            false,
        )
        .unwrap(),
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(crate::command::workflow_live::stage_landing_scope(Some(records.clone()), async {
        let tool = archon_tools::audit_landing::current().unwrap();
        let task = serde_json::json!({"task_id":"TASK-X-010","file_name":"TASK-X-010-body.md","implements":["AC-X-001"],"source_sections":["0"]});
        let receipt = tool.land(serde_json::json!({"subject":"TASK-X-010","task":task})).unwrap();
        assert!(receipt.contains("records_landed"));
        tool.complete(&serde_json::json!({"records_landed":1})).unwrap();
    }));
    let candidate = records
        .compose(&serde_json::json!({"records_landed":1}))
        .unwrap();
    std::fs::write(
        tasks.join(TASK_SKELETON_FILE),
        serde_json::to_vec(&candidate).unwrap(),
    )
    .unwrap();
    let result = freeze_skeleton(temp.path(), &tasks, &temp.path().join("prds/PRD-X.md")).unwrap();
    assert!(!result.skeleton_digest.is_empty());
    let frozen: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(&std::fs::read(tasks.join(TASK_SKELETON_FILE)).unwrap()).unwrap();
    assert_eq!(frozen.tasks[0].source_sections, ["0"]);
}

#[tokio::test]
#[allow(non_snake_case)]
async fn R6_P5_legacy_skeleton_defects_refuse_acceptance_republish_atomically() {
    use crate::command::workflow_task_set::reauthor::{
        AuthorScope,
        test_client::{ScriptedAuthorJudge, command_entry},
    };
    use crate::command::workflow_task_set::republish::{
        ReauthorRequest, reauthor_and_republish, test_fixture::frozen_set_in_proven,
    };
    use archon_workflow::task_set_contract::{
        FreezeGateMode, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
    };
    for constraint in [false, true] {
        let set = frozen_set_in_proven(
            &[("AC-F-001", "jq -e '.a == true' out.json", true)],
            FreezeGateMode::Enforce,
            if constraint {
                "\n- QX-001: bound work\n"
            } else {
                ""
            },
        );
        if !constraint {
            let path = set.tasks.join(TASK_SKELETON_FILE);
            let mut skeleton: archon_workflow::task_skeleton::TaskSkeleton =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let id = skeleton.tasks[0].implements[0].clone();
            skeleton.tasks[0].implements.push(id);
            let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
            std::fs::write(path, &bytes).unwrap();
            let digest = content_digest(&bytes);
            let lock_path = set.tasks.join(TASK_SKELETON_LOCK_FILE);
            let mut lock: archon_workflow::task_skeleton::TaskSkeletonLock =
                serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
            lock.digest = digest.clone();
            std::fs::write(lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
            let mut pin = set.pin();
            pin.skeleton_digest = Some(digest);
            std::fs::write(set.pin_path(), serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
        }
        let before = set.chain_bytes();
        let named = ["AC-F-001".to_owned()].into_iter().collect();
        let client = ScriptedAuthorJudge::new(
            |entry, _| command_entry(entry, "jq -e '.a == true and .b == 1' out.json"),
            |_, _| true,
        );
        let scope =
            AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd)
                .unwrap();
        let error = reauthor_and_republish(
            &client,
            ReauthorRequest {
                project_root: set.project.path(),
                tasks_root: &set.tasks,
                prd_path: &set.prd,
                ids: &named,
                gate: set.gate(),
                trigger: "test",
            },
            &scope,
        )
        .await
        .expect_err("legacy skeleton now fails the skeleton gate");
        let text = format!("{error:#}");
        assert!(
            text.contains("freeze_skeleton gate refused") && text.contains("nothing was written"),
            "{text}"
        );
        assert!(
            text.contains(if constraint {
                "in no task's implements"
            } else {
                "twice in implements"
            }),
            "{text}"
        );
        assert_eq!(set.chain_bytes(), before);
    }
}
