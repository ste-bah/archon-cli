use super::*;

#[test]
fn identical_bounded_check_reuses() {
    let assessment = assess("test -f feature.txt", "abc", "logic-1", "env-1");
    assert!(assessment.reusable, "{assessment:?}");
    assert_eq!(assessment.reason, "all inputs are host-proven identical");
}

#[test]
fn changed_check_text_reruns() {
    let old = key("test -f feature.txt", "abc", "logic-1", "env-1");
    let changed = key("test -s feature.txt", "abc", "logic-1", "env-1");
    assert_ne!(old, changed);
}

#[test]
fn changed_repository_closure_reruns() {
    let old = key("test -f feature.txt", "tree-a", "logic-1", "env-1");
    let changed = key("test -f feature.txt", "tree-b", "logic-1", "env-1");
    assert_ne!(old, changed);
}

#[test]
fn unbounded_check_is_volatile() {
    let assessment = assess("cargo test", "tree", "logic-1", "env-1");
    assert!(!assessment.reusable);
    assert_eq!(assessment.reason, "check read closure is unbounded");
}

#[test]
fn logic_upgrade_reruns() {
    let old = key("test -f feature.txt", "tree", "logic-1", "env-1");
    let upgraded = key("test -f feature.txt", "tree", "logic-2", "env-1");
    assert_ne!(old, upgraded);
}

#[test]
fn reuse_decision_and_reason_are_recordable() {
    let assessment = assess("test -f feature.txt", "tree", "logic-1", "env-1");
    let record = serde_json::json!({
        "reused": assessment.reusable,
        "why": assessment.reason,
        "evidence": "probe-results/original.json"
    });
    assert_eq!(record["reused"], true);
    assert_eq!(record["why"], "all inputs are host-proven identical");
    assert_eq!(record["evidence"], "probe-results/original.json");
}

fn key(check: &str, tree: &str, logic: &str, environment: &str) -> String {
    super::reuse_key(check, tree, logic, environment)
}

#[test]
fn shell_globs_and_expansions_are_volatile() {
    assert!(!assess("test -f *.txt", "tree", "logic-1", "env-1").reusable);
    assert!(!assess("test -f $FILE", "tree", "logic-1", "env-1").reusable);
}
