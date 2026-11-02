use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::command::workflow_task_set::judge::continuation_tests::{Scripted, contract};

#[tokio::test]
async fn an_incomplete_reauthor_judge_returns_decided_entries_and_retries_only_missing_ones() {
    let mut asked = contract();
    let mut second = asked.acceptance[0].clone();
    second.id = "AC-X-002".into();
    asked.acceptance.push(second);
    let client = Scripted::new(vec![
        Ok((
            r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"the neutral check is sound"}]}"#,
            Some("end_turn"),
        )),
        Ok(("no structured reply", Some("end_turn"))),
        Ok(("no structured reply", Some("end_turn"))),
        Ok(("no structured reply", Some("end_turn"))),
        Ok(("no structured reply", Some("end_turn"))),
        Ok(("no structured reply", Some("end_turn"))),
        Ok(("no structured reply", Some("end_turn"))),
        Ok(("no structured reply", Some("end_turn"))),
    ]);
    let error = crate::command::workflow_task_set::judge::judge_prompted(
        &client,
        asked.clone(),
        |_| Ok("NEUTRAL PROMPT".to_string()),
        "sonnet",
        |_| Ok(()),
        None,
    )
    .await
    .expect_err("the second id never receives a verdict");

    let mut retry = JudgeRetryCandidates::default();
    let mut feedback = BTreeMap::from([
        ("AC-X-001".to_string(), Vec::new()),
        ("AC-X-002".to_string(), Vec::new()),
    ]);
    let mut pending = BTreeSet::from(["AC-X-001".to_string(), "AC-X-002".to_string()]);
    let mut reused = BTreeMap::new();
    let mut unproven = BTreeMap::new();
    let decided = hold_judge_failure(
        error,
        &asked,
        &mut retry,
        &mut feedback,
        (&mut pending, &mut reused, &mut unproven),
        &mut BTreeMap::new(),
    )
    .expect("typed incomplete judge");

    assert_eq!(decided.acceptance.len(), 1);
    assert_eq!(decided.acceptance[0].id, "AC-X-001");
    assert_eq!(
        decided.acceptance[0].judgment.verdict,
        archon_workflow::task_set_contract::JudgeDecision::Accepted
    );
    assert!(retry.take("AC-X-001").is_none());
    assert_eq!(
        retry
            .take("AC-X-002")
            .expect("undecided entry is retried")
            .id,
        "AC-X-002"
    );
}
