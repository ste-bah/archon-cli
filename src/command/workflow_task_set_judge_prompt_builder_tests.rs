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
            r#"{"decisions":[{"id":"AC-X-001","kind":"state","cited_paths":[],"verdict":"accepted","counterexample":"none","reason":"the neutral check is sound"}]}"#,
            Some("end_turn"),
        )),
        Ok((
            r#"{"decisions":[{"id":"AC-X-002","kind":"state","cited_paths":[],"verdict":"accepted","counterexample":"none","reason":"the neutral check is sound"}]}"#,
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

fn checks_in_prompt(subject: &AcceptanceContract, roots: &JudgeRoots) -> serde_json::Value {
    let prompt = batched_judge_prompt_with_roots(subject, roots).unwrap();
    assert!(prompt.contains("Each covers entry's PRD text is in covers_text; refute (kind state) when a state that violates that text on the path the check drives still passes the check, and name that id in reason."));
    serde_json::from_str(prompt.rsplit_once("Checks: ").unwrap().1).unwrap()
}

#[test]
fn covered_requirement_text_is_supplied_for_each_check() {
    let mut subject = contract();
    subject.acceptance[0].covers = vec!["REQ-X-001".into()];
    let mut supplementary = subject.acceptance[0].clone();
    supplementary.id = "SUP-REQ-X-001".into();
    subject.supplementary.push(supplementary);
    let text = "Return the complete input unchanged.";
    let roots = JudgeRoots {
        covered_texts: archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(
            &format!("- REQ-X-001: {text}"),
        ),
        ..Default::default()
    };
    let checks = checks_in_prompt(&subject, &roots);
    for check in checks.as_array().unwrap() {
        assert_eq!(check["covers"], serde_json::json!(["REQ-X-001"]));
        assert_eq!(check["covers_text"]["REQ-X-001"], text);
    }
}

#[test]
fn missing_covered_requirement_text_is_empty() {
    let mut subject = contract();
    subject.acceptance[0].covers = vec!["REQ-X-001".into(), "REQ-X-002".into()];
    let roots = JudgeRoots {
        covered_texts: BTreeMap::from([("REQ-X-001".into(), "Keep the input.".into())]),
        ..Default::default()
    };
    let checks = checks_in_prompt(&subject, &roots);
    assert_eq!(checks[0]["covers_text"]["REQ-X-001"], "Keep the input.");
    assert_eq!(checks[0]["covers_text"]["REQ-X-002"], "");
}
