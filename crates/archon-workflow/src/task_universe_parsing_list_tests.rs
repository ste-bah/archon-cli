use super::list_items::list_item_text;
use super::*;

/// The live failure. Every task in the PRD writes acceptance criteria as a
/// numbered list, so all fifteen parsed to zero criteria and the no-op proof
/// rejected each one as "acceptance_criteria is missing or empty".
#[test]
fn numbered_acceptance_criteria_are_parsed() {
    let raw = "\
# TASK-TDL-020

## Acceptance Criteria

1. Validation reports record every required check id.
2. Reports fail closed when evidence is absent.
12. Every filtered test runs at least one non-ignored test.

## Focused Tests
";

    let criteria = declared_task_section_items(raw, "acceptance criteria");

    assert_eq!(criteria.len(), 3, "got {criteria:?}");
    assert!(
        criteria
            .iter()
            .any(|c| c.starts_with("Validation reports record"))
    );
    assert!(
        criteria
            .iter()
            .any(|c| c.starts_with("Every filtered test runs"))
    );
    assert!(
        criteria.iter().all(|c| !c.starts_with(char::is_numeric)),
        "the ordinal is a marker, not content: {criteria:?}"
    );
}

/// Bullets must keep working — the PRD may mix styles.
#[test]
fn bullet_and_paren_markers_still_parse() {
    let raw = "\
## Acceptance Criteria

- Dash bullet criterion.
* Star bullet criterion.
+ Plus bullet criterion.
3) Paren-numbered criterion.

## Next
";

    let criteria = declared_task_section_items(raw, "acceptance criteria");

    assert_eq!(criteria.len(), 4, "got {criteria:?}");
    assert!(criteria.iter().any(|c| c == "Paren-numbered criterion."));
}

/// A section with no list yields nothing rather than swallowing prose.
#[test]
fn prose_without_list_markers_is_not_a_criterion() {
    let raw = "\
## Acceptance Criteria

This section is prose and declares no enumerated criteria.

## Next
";

    assert!(declared_task_section_items(raw, "acceptance criteria").is_empty());
}

/// A bare number is a marker only when a delimiter follows it.
#[test]
fn a_number_without_a_delimiter_is_not_a_list_item() {
    assert_eq!(list_item_text("2024 was the baseline year"), None);
    assert_eq!(list_item_text("1."), None);
    assert_eq!(list_item_text("1. real criterion"), Some("real criterion"));
}

/// Focused tests written in a fenced block are declared commands.
///
/// `declared_task_section_items` reads list items only. Task bodies that put
/// their commands in a ```sh block therefore contributed nothing, so the task
/// universe recorded no focused tests, the v3 author brief said "no task
/// declares any focused test", and the authored workflow passed no focusedTests
/// anywhere — every task verified more weakly than its own body specified.
///
/// The traceability reader learned this already: "a fenced block is a
/// reasonable way to write a list of commands and only this reader disagreed".
/// That lesson was applied to one parser and not this one.
#[test]
fn focused_tests_in_a_fenced_block_are_declared() {
    let raw = "\
# TASK-X-010

## Focused Tests

Both commands read the exact output file; each must exit 0:

```sh
test -s src/alpha.txt
```

```sh
[ \"$(head -n 1 src/alpha.txt)\" = \"alpha ready\" ]
```

## Acceptance Mapping
";
    let tests = super::declared_focused_tests(raw);
    assert_eq!(
        tests.len(),
        2,
        "both fenced commands must be declared: {tests:?}"
    );
    assert!(tests.iter().any(|t| t.starts_with("test -s")), "{tests:?}");
}

/// Bulleted focused tests keep working, and the two forms combine.
#[test]
fn bulleted_and_fenced_focused_tests_are_both_declared() {
    let raw = "\
# TASK-X-010

## Focused Tests

- `grep -qx 'alpha ready' src/alpha.txt`

```sh
test -s src/alpha.txt
```
";
    let tests = super::declared_focused_tests(raw);
    assert_eq!(tests.len(), 2, "{tests:?}");
}

/// A fence outside the section is not a focused test.
#[test]
fn a_fenced_block_outside_the_focused_tests_section_is_ignored() {
    let raw = "\
# TASK-X-010

## Notes

```sh
rm -rf /
```

## Focused Tests

- `test -s src/alpha.txt`
";
    let tests = super::declared_focused_tests(raw);
    assert_eq!(tests.len(), 1, "{tests:?}");
    assert!(!tests.iter().any(|t| t.contains("rm -rf")), "{tests:?}");
}

/// Diagnostic: what the universe would declare for a real task file.
///
/// ARCHON_TASK_FILE=<path> cargo test -p archon-workflow --lib \
///   focused_tests_declared_by_a_real_body -- --ignored --nocapture
#[test]
#[ignore = "diagnostic; requires ARCHON_TASK_FILE"]
fn focused_tests_declared_by_a_real_body() {
    let path = std::env::var("ARCHON_TASK_FILE").expect("set ARCHON_TASK_FILE");
    let raw = std::fs::read_to_string(&path).expect("read task file");
    for command in super::declared_focused_tests(&raw) {
        println!("DECLARED: {command}");
    }
}
