//! Issue-118: how an excused red test is recorded on the branch result, and
//! how its names are written so none is ever cut.

use serde_json::Value;

use super::{BranchJudgement, NOT_RUN_DATA_KEY, UNOWNED_RED_DATA_KEY, UNOWNED_RED_GAP_ID};
use crate::v2::WorkflowV2Result;

/// Record the excused tests on the branch result: the host gap and the
/// typed list. The verdict itself is left to the caller. The gap's id carries
/// a digest of the tests it names, and its text opens with them, so the same
/// tests red again read as the same gap and different ones never do.
pub(crate) fn record_unowned_red_tests(result: &mut WorkflowV2Result, judged: &BranchJudgement) {
    if !judged.not_run.is_empty() {
        let mut data = result.data.as_object().cloned().unwrap_or_default();
        data.insert(
            NOT_RUN_DATA_KEY.to_string(),
            serde_json::json!(judged.not_run),
        );
        result.data = Value::Object(data);
    }
    let tests = judged.excused.as_slice();
    if tests.is_empty() {
        return;
    }
    let base: String = tests[0].run_base.chars().take(12).collect();
    let mut ids: Vec<&str> = tests.iter().map(|test| test.test_id.as_str()).collect();
    ids.sort_unstable();
    let digest = blake3::hash(ids.join("\n").as_bytes()).to_hex();
    let names: Vec<String> = tests.iter().map(|test| test.test_id.clone()).collect();
    let mut files: Vec<&str> = tests
        .iter()
        .flat_map(|test| test.files.iter().map(String::as_str))
        .collect();
    files.sort_unstable();
    files.dedup();
    let id = format!("{UNOWNED_RED_GAP_ID}-{}", &digest[..8]);
    // A reused outcome judged again already carries it.
    if result.residual_gaps.iter().any(|gap| gap.id == id) {
        return;
    }
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id,
        description: format!(
            "{}: red on this tree and already red on the run's base commit {base}, where the \
             host ran the verifier's own command; their files lie outside this branch's \
             writable scope, so they do not refuse its verdict, and they are still owed. \
             Files: {}",
            grouped_names(&names),
            files.join(", ")
        ),
        severity: Some("medium".to_string()),
    });
    let mut data = result.data.as_object().cloned().unwrap_or_default();
    data.insert(
        UNOWNED_RED_DATA_KEY.to_string(),
        serde_json::to_value(tests).unwrap_or_default(),
    );
    result.data = Value::Object(data);
}

/// Red test ids, grouped under their common module:
/// `` `a::b::{t1, t2}`, `c::t3` `` -- compact, so a round's claim carries
/// every name before any file list.
pub fn grouped_names(ids: &[String]) -> String {
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for id in ids {
        let (module, leaf) = id.rsplit_once("::").unwrap_or(("", id));
        match groups.iter_mut().find(|(seen, _)| *seen == module) {
            Some((_, leaves)) => leaves.push(leaf),
            None => groups.push((module, vec![leaf])),
        }
    }
    groups
        .iter()
        .map(
            |(module, leaves)| match (module.is_empty(), leaves.as_slice()) {
                (true, _) => leaves
                    .iter()
                    .map(|l| format!("`{l}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                (false, [one]) => format!("`{module}::{one}`"),
                (false, many) => format!("`{module}::{{{}}}`", many.join(", ")),
            },
        )
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether a gap id is the host's record of excused red tests (flagged or
/// not).
pub fn is_unowned_red_gap_id(id: &str) -> bool {
    id.trim()
        .trim_start_matches(crate::v2::verification::UNOWNED_PATH_GAP_PREFIX)
        .starts_with(UNOWNED_RED_GAP_ID)
}
