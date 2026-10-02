//! Batch E: a refused project-input landing is owed to its tasks until a
//! later landing applies the same path for them.
use super::*;

fn line(item: &str, tasks: &[&str], path: &str, outcome: &str, at: i64) -> ProjectInputLanding {
    ProjectInputLanding {
        stage_id: "stage".into(),
        item_id: item.into(),
        task_ids: tasks.iter().map(|t| t.to_string()).collect(),
        path: path.into(),
        outcome: outcome.into(),
        before: "b".into(),
        after: "a".into(),
        reason: if outcome == "refused" {
            "stale baseline".into()
        } else {
            String::new()
        },
        at,
        created_dirs: Vec::new(),
    }
}

fn store(lines: &[ProjectInputLanding]) -> (tempfile::TempDir, WorkflowV2ResultStore) {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("write-coordination/project-inputs.jsonl");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    let text: String = lines
        .iter()
        .map(|l| format!("{}\n", serde_json::to_string(l).unwrap()))
        .collect();
    std::fs::write(log, text).unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("v2"));
    (dir, store)
}

#[test]
fn a_refusal_is_owed_per_task_until_a_later_landing_applies_its_path() {
    let (_dir, store) = store(&[
        line(
            "fix-1",
            &["TASK-A", "TASK-B"],
            "data/registry.json",
            "refused",
            10,
        ),
        line(
            "fix-1",
            &["TASK-A", "TASK-B"],
            "data/index.json",
            "refused",
            10,
        ),
        // TASK-A's later branch applied the registry; the index stays owed.
        line("fix-2", &["TASK-A"], "data/registry.json", "applied", 20),
        // Decided after the cut: never read.
        line("fix-3", &["TASK-B"], "data/registry.json", "applied", 40),
        line("fix-4", &["TASK-C"], "data/other.json", "refused", 50),
    ]);
    let gaps = refused_input_gaps(&store, Some(30));
    let owed: Vec<(&str, &str)> = gaps
        .iter()
        .map(|g| (g.recorded_by.as_str(), g.id.as_str()))
        .collect();
    assert_eq!(
        owed,
        [
            ("project-inputs:stage/fix-1", "TASK-A"),
            ("project-inputs:stage/fix-1", "TASK-B"),
        ]
    );
    assert!(
        gaps.iter()
            .all(|g| g.severity == ResidualSeverity::High && g.host_built)
    );
    assert!(gaps[0].description.contains("data/index.json"));
    assert!(
        !gaps[0].description.contains("data/registry.json"),
        "{}",
        gaps[0].description
    );
    assert!(
        gaps[1]
            .description
            .contains("data/index.json, data/registry.json")
    );
    assert_eq!(
        gaps[1].unit_tasks,
        ["TASK-B".to_string()].into_iter().collect()
    );
    // Without a cut every decision counts.
    let all = refused_input_gaps(&store, None);
    assert_eq!(all.len(), 2 + 1, "{all:?}");
    assert!(
        all.iter()
            .all(|g| !(g.id == "TASK-B" && g.description.contains("registry")))
    );
}

#[test]
fn no_log_owes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("v2"));
    assert!(refused_input_gaps(&store, None).is_empty());
}
