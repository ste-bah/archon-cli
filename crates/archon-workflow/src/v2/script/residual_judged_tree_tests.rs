use super::*;
pub(super) fn record(world: &World, original: &WorkflowV2CallRecord) -> WorkflowV2CallRecord {
    let mut record = original.clone();
    if !world.root().join(".git").exists() {
        return record;
    }
    let Some(views) = record
        .result
        .data
        .get_mut("outcomes")
        .and_then(Value::as_array_mut)
    else {
        return record;
    };
    for view in views {
        if view
            .pointer("/result/data/baseline_red_tests")
            .and_then(Value::as_array)
            .is_some_and(|red| !red.is_empty())
            && view.pointer("/result/data/judged_commit").is_none()
        {
            let output = crate::write_coordinator::worktree_isolation::run_git(
                &["rev-parse", "HEAD"],
                world.root(),
            )
            .unwrap();
            let commit = String::from_utf8(output.stdout).unwrap().trim().to_string();
            view["result"]["data"]["judged_commit"] = json!(commit);
        }
    }
    record
}
