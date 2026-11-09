use super::*;
use crate::v2::write::test_baseline_run_base::{self as host, HostRunVerdict, Tree};

#[test]
fn old_execution_logic_command_entries_are_not_reused() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let verdict: CommandBaseline = serde_json::from_value(serde_json::json!({
        "command": "check", "base_commit": "abc", "exit_code": 1,
        "timed_out": false, "duration_ms": 1, "failing_tests": []
    }))
    .unwrap();
    for inputs in [None, Some("seed")] {
        let key = match inputs {
            Some(digest) => format!("check\0project-inputs:{digest}"),
            None => "check".to_string(),
        };
        let hash = blake3::hash(format!("execution-logic-2\0{key}").as_bytes()).to_hex();
        let old = root(&store)
            .join("cache")
            .join(format!("abc-{}.json", &hash[..16]));
        write_json(&old, &verdict).unwrap();
        assert!(cached_command_for(&store, "abc", "check", inputs).is_none());
        cache_command_for(&store, &verdict, inputs);
        assert!(
            cached_command_for(&store, "abc", "check", inputs)
                .unwrap()
                .cached
        );
        assert!(old.exists());
    }
}

#[test]
fn old_execution_logic_base_and_judged_entries_are_not_reused() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let verdict = HostRunVerdict {
        command: "check".into(),
        commit: "abc".into(),
        exit_code: Some(1),
        verbatim: true,
        isolated: true,
        ..Default::default()
    };
    let mut value = serde_json::to_value(&verdict).unwrap();
    value["identity_digits_preserved"] = serde_json::Value::Bool(true);
    let hash = blake3::hash(b"execution-logic-2\0check").to_hex();
    for (tree, tag) in [(Tree::RunBase, "base"), (Tree::Judged, "judged")] {
        let old = root(&store)
            .join("run-base")
            .join(format!("{tag}-abc-{}.json", &hash[..16]));
        write_json(&old, &value).unwrap();
        assert!(host::cached(&store, tree, "abc", "check").is_none());
        host::cache(&store, tree, &verdict);
        assert_eq!(
            host::cached(&store, tree, "abc", "check"),
            Some(verdict.clone())
        );
        assert!(old.exists());
    }
}
