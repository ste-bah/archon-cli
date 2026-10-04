//! Every copy of a duplicated field is read by the derived reader, so each
//! copy's leaf defects must count on their own: repairing any one copy lowers
//! the count, even when another copy has the same problem.
use super::*;
use archon_workflow::task_skeleton::TaskSkeleton;

const HEAD: &str = r#""schema_version":1,"acceptance_digest":"d""#;

fn task(body: &str) -> String {
    format!(r#"{{{HEAD},"tasks":[{body}]}}"#)
}

fn counts(steps: &[String]) -> Vec<usize> {
    steps
        .iter()
        .map(|step| element_shape_defects(step.as_bytes(), &TASK_SHAPE).len())
        .collect()
}

fn strictly_decreasing(steps: &[String]) {
    let counts = counts(steps);
    assert!(
        counts.windows(2).all(|pair| pair[1] < pair[0]),
        "{counts:?} for {steps:?}"
    );
    let last = steps.last().unwrap();
    assert_eq!(
        counts.last() == Some(&0),
        serde_json::from_str::<TaskSkeleton>(last).is_ok(),
        "{last}"
    );
}

#[test]
fn workflow_freeze_round13_repairing_either_invalid_copy_lowers_the_count() {
    let both = task(r#"{"task_id":5,"task_id":6,"file_name":"f"}"#);
    strictly_decreasing(&[
        both.clone(),
        task(r#"{"task_id":"T","task_id":6,"file_name":"f"}"#),
        task(r#"{"task_id":"T","task_id":"T","file_name":"f"}"#),
        task(r#"{"task_id":"T","file_name":"f"}"#),
    ]);
    strictly_decreasing(&[
        both,
        task(r#"{"task_id":5,"task_id":"T","file_name":"f"}"#),
        task(r#"{"task_id":"T","task_id":"T","file_name":"f"}"#),
    ]);
}

#[test]
fn workflow_freeze_round13_overwritten_container_copies_count_their_leaves() {
    let copy = |kind: bool| {
        let kind = if kind { r#""kind":"k","# } else { "" };
        format!(r#""deliverable_contracts":[{{{kind}"artifact_path":"a"}}]"#)
    };
    let pair = |first: bool, second: bool| {
        task(&format!(
            r#"{{"task_id":"T","file_name":"f",{},{}}}"#,
            copy(first),
            copy(second)
        ))
    };
    strictly_decreasing(&[
        pair(false, false),
        pair(true, false),
        pair(true, true),
        task(&format!(
            r#"{{"task_id":"T","file_name":"f",{}}}"#,
            copy(true)
        )),
    ]);
    strictly_decreasing(&[pair(false, false), pair(false, true), pair(true, true)]);
}

#[test]
fn workflow_freeze_round13_every_overwritten_copy_has_its_own_identity() {
    // Three invalid copies: fixing the first moves the reader from a type error
    // to a duplicate error; fixing the middle one is also a real repair.
    strictly_decreasing(&[
        task(r#"{"task_id":1,"task_id":2,"task_id":3,"file_name":"f"}"#),
        task(r#"{"task_id":"T","task_id":2,"task_id":3,"file_name":"f"}"#),
        task(r#"{"task_id":"T","task_id":"T","task_id":3,"file_name":"f"}"#),
        task(r#"{"task_id":"T","task_id":"T","task_id":"T","file_name":"f"}"#),
    ]);
}

#[test]
fn workflow_freeze_round13_duplicates_inside_an_overwritten_copy_count_on_their_own() {
    let inner = |dup: bool| {
        let extra = if dup { r#""task_id":"T","# } else { "" };
        format!(r#"{{{extra}"task_id":"T","file_name":"f"}}"#)
    };
    let doc = |first: bool, second: bool| {
        format!(
            r#"{{{HEAD},"tasks":[{}],"tasks":[{}]}}"#,
            inner(first),
            inner(second)
        )
    };
    strictly_decreasing(&[doc(true, true), doc(false, true), doc(false, false)]);
    strictly_decreasing(&[doc(true, true), doc(true, false), doc(false, false)]);
}

#[test]
fn workflow_freeze_round13_unknown_field_twice_in_a_closed_struct_counts_each_copy() {
    let contract = |extra: &str| {
        task(&format!(
            r#"{{"task_id":"T","file_name":"f","deliverable_contracts":[{{"kind":"k","artifact_path":"a"{extra}}}]}}"#
        ))
    };
    strictly_decreasing(&[
        contract(r#","zz":1,"zz":1"#),
        contract(r#","zz":1"#),
        contract(""),
    ]);
    // Ignored fields of an open struct keep serde's behaviour: no defect.
    let open = task(r#"{"task_id":"T","file_name":"f","zz":1,"zz":{"task_id":1,"task_id":2}}"#);
    assert!(serde_json::from_str::<TaskSkeleton>(&open).is_ok());
    assert!(element_shape_defects(open.as_bytes(), &TASK_SHAPE).is_empty());
}
