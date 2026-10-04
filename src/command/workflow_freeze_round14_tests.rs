//! Three or more copies: the derived reader reports the first invalid copy,
//! or the first repeated copy. Every copy is one more problem to remove, so
//! deleting, renaming or repairing any copy that changes the reader's verdict
//! must change the count, however later copies are renumbered.
use super::*;
use archon_workflow::task_skeleton::TaskSkeleton;

const HEAD: &str = r#""schema_version":1,"acceptance_digest":"d""#;

fn task(body: &str) -> String {
    format!(r#"{{{HEAD},"tasks":[{{{body},"file_name":"f"}}]}}"#)
}

fn verdict(document: &str) -> (usize, Result<(), String>) {
    let reader = serde_json::from_str::<TaskSkeleton>(document)
        .map(drop)
        .map_err(|e| e.to_string().split(" at line ").next().unwrap().to_string());
    let count = element_shape_defects(document.as_bytes(), &TASK_SHAPE).len();
    assert_eq!(count == 0, reader.is_ok(), "{document}");
    (count, reader)
}

fn repair_lowers(before: &str, after: &str) {
    let (b, rb) = verdict(before);
    let (a, ra) = verdict(after);
    assert!(
        a < b,
        "{b} -> {a} defects; reader {rb:?} -> {ra:?}\n{before}\n{after}"
    );
}

#[test]
fn workflow_freeze_round14_deleting_the_first_of_three_copies_lowers_the_count() {
    repair_lowers(
        &task(r#""task_id":"T","task_id":2,"task_id":3"#),
        &task(r#""task_id":2,"task_id":3"#),
    );
    repair_lowers(
        &task(r#""task_id":"T","task_id":2,"task_id":"U""#),
        &task(r#""task_id":2,"task_id":"U""#),
    );
}

#[test]
fn workflow_freeze_round14_renaming_a_copy_to_an_ignored_key_lowers_the_count() {
    repair_lowers(
        &task(r#""task_id":"T","task_id":2,"task_id":3"#),
        &task(r#""ignored":"T","task_id":2,"task_id":3"#),
    );
}

#[test]
fn workflow_freeze_round14_nested_three_copy_parents_lower_the_count() {
    let valid = r#""tasks":[{"task_id":"T","file_name":"f"}]"#;
    let invalid = r#""tasks":[{"task_id":"T","task_id":2,"file_name":"f"}]"#;
    let doc = |copies: &[&str]| format!("{{{HEAD},{}}}", copies.join(","));
    repair_lowers(&doc(&[valid, invalid, valid]), &doc(&[invalid, valid]));
    repair_lowers(
        &doc(&[invalid, invalid, invalid]),
        &doc(&[invalid, invalid]),
    );
}

#[test]
fn workflow_freeze_round14_every_repeated_copy_is_its_own_defect() {
    // Each extra copy must be removed before the reader accepts the struct.
    let counts: Vec<_> = (1..=4)
        .map(|n| verdict(&task(&vec![r#""task_id":"T""#; n].join(","))).0)
        .collect();
    assert_eq!(counts, [0, 1, 2, 3]);
}

#[test]
fn workflow_freeze_round14_messages_number_copies_from_one() {
    let defects = element_shape_defects(
        task(r#""task_id":1,"task_id":2,"task_id":"T""#).as_bytes(),
        &TASK_SHAPE,
    );
    let messages: Vec<_> = defects.iter().map(|d| d.message.as_str()).collect();
    for expected in [
        "copy 1 of 3",
        "copy 2 of 3",
        "copy 2 repeats",
        "copy 3 repeats",
    ] {
        assert!(
            messages.iter().any(|m| m.contains(expected)),
            "{expected}: {messages:?}"
        );
    }
}
