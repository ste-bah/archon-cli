use super::*;
use crate::command::workflow_task_set::judge::continuation_tests::{Scripted, contract};

#[tokio::test]
async fn acceptance_reask_uses_the_acceptance_prompt_for_the_missing_id() {
    let mut subject = contract();
    let mut second = subject.acceptance[0].clone();
    second.id = "AC-X-002".into();
    subject.acceptance.push(second);
    let client = Scripted::new(vec![
        Ok((
            r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"the neutral check is sound"}]}"#,
            Some("end_turn"),
        )),
        Ok((
            r#"{"decisions":[{"id":"AC-X-002","verdict":"accepted","counterexample":"none","reason":"the neutral check is sound"}]}"#,
            Some("end_turn"),
        )),
    ]);
    judge_batch(
        &client,
        subject,
        "sonnet",
        require_judged_prose,
        None,
        &JudgeRoots::default(),
    )
    .await
    .expect("both checks receive decisions");

    let calls = client.calls();
    let reask = calls[1][0]["content"].as_str().unwrap();
    assert!(reask.contains("Adversarially judge every acceptance check below"));
    assert!(reask.contains("AC-X-002"));
    assert!(!reask.contains("AC-X-001"));
}
