use super::*;

#[test]
fn block_sequence_implements_is_read() {
    let raw = "```yaml\ntask_id: TASK-X-001\nimplements:\n  - REQ-DL-001\n```\n";
    let binding = parse_task_binding(raw, "tests/TASK-X-001.md").expect("block sequence parses");
    assert_eq!(binding.implements, ["REQ-DL-001"]);
}

#[test]
fn quoted_and_plain_task_ids_have_the_same_value() {
    for scalar in ["TASK-X-001", "\"TASK-X-001\"", "'TASK-X-001'"] {
        let raw = format!("```yaml\ntask_id: {scalar}\nimplements: []\n```\n");
        let binding = parse_task_binding(&raw, "tasks/TASK-X-001.md").expect("parses");
        assert_eq!(binding.task_id, "TASK-X-001");
    }
}

#[test]
fn quoted_slugged_task_id_remains_noncanonical() {
    let raw = "```yaml\ntask_id: \"TASK-X-001-slug\"\nimplements: []\n```\n";
    let error = parse_task_binding(raw, "tasks/TASK-X-001-slug.md").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("task_id 'TASK-X-001-slug' is noncanonical")
    );
}

#[test]
fn quoted_flow_sequence_items_are_decoded_by_yaml() {
    let raw = "```yaml\ntask_id: TASK-X-001\nimplements: ['REQ-A-001', \"REQ-B-002\"]\n```\n";
    let binding = parse_task_binding(raw, "tasks/TASK-X-001.md").expect("parses");
    assert_eq!(binding.implements, ["REQ-A-001", "REQ-B-002"]);
}

#[test]
fn malformed_implements_keeps_the_existing_refusal_message() {
    let raw = "```yaml\ntask_id: TASK-X-001\nimplements: [REQ-X-001\n```\n";
    let error = parse_task_binding(raw, "tasks/broken.md").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("must be a single-line flow sequence")
    );
    assert!(error.to_string().contains("found `[REQ-X-001`"));
}

#[test]
fn required_tools_block_sequence_is_read() {
    let raw = "\
# TASK-W

```yaml
task_id: TASK-W-001
implements: [REQ-4]
required_tools:
  - cargo
  - lizard
```

## Focused Tests

- `cargo test`
";
    let binding = parse_task_binding(raw, "tasks/TASK-W-001.md").expect("parses");
    assert_eq!(binding.implements, vec!["REQ-4"]);
    assert_eq!(binding.required_tools, ["cargo", "lizard"]);
}

#[test]
fn malformed_required_tools_scalar_does_not_fail_the_task() {
    let raw = "```yaml\ntask_id: TASK-W-001\nimplements: [REQ-4]\nrequired_tools: cargo\n```\n";
    let binding = parse_task_binding(raw, "tasks/TASK-W-001.md").expect("tolerated");
    assert_eq!(binding.implements, ["REQ-4"]);
    assert!(binding.required_tools.is_empty());
}
