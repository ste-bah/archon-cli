use super::*;
use crate::command::workflow_task_set::judge::continuation_tests::{Scripted, contract};

fn candidate(id: &str) -> Candidate {
    let evidence = Evidence {
        commit: "base".into(),
        exit_code: Some(1),
        stderr: "baseline output".into(),
        stdout: String::new(),
        reused: false,
        reuse_why: None,
        evidence_reference: None,
    };
    Candidate {
        id: id.into(),
        shown: serde_json::json!({"id": id, "criterion": "A check has a neutral property.", "baseline": evidence}),
        evidence,
        key: format!("key-{id}"),
    }
}

#[tokio::test]
async fn a_missing_passability_id_is_reasked_with_the_passability_prompt() {
    let mut subject = contract();
    let mut second = subject.acceptance[0].clone();
    second.id = "AC-X-002".into();
    subject.acceptance.push(second);
    let candidates = [candidate("AC-X-001"), candidate("AC-X-002")];
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

    ask(
        &client,
        "sonnet",
        &subject,
        &[&candidates[0], &candidates[1]],
    )
    .await
    .expect("both checks receive decisions");

    let calls = client.calls();
    let reask = calls[1][0]["content"].as_str().expect("user prompt");
    assert!(
        reask.contains("Each acceptance check below already ran once"),
        "missing passability prompt sentence: {reask}"
    );
    assert!(
        !reask.contains("Judge supplied named_files content as written"),
        "acceptance judge prompt leaked into passability re-ask: {reask}"
    );
    assert!(
        reask.contains("AC-X-002"),
        "re-ask must contain only the missing id: {reask}"
    );
    assert!(
        !reask.contains("AC-X-001"),
        "decided id must not be re-asked: {reask}"
    );
}
