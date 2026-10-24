//! A first reply cut off at the output limit. Its stop reason is read before
//! its text: a length stop is continued whatever the partial content, as a
//! reply cut off inside its document is (provider guidance: read the finish
//! reason before parsing). Only text that already contradicts a JSON
//! document cannot be extended; that reply is asked afresh.

use super::*;

/// Supersedes Round 3 decision D (prose with no document was asked afresh):
/// a length stop before the first brace is continued with the same reply.
#[tokio::test]
async fn a_truncated_reply_with_no_document_is_continued_with_the_same_reply() {
    let fenced = format!("```json\n{ACCEPTED}\n```");
    let client = Scripted::new(vec![
        Ok(("Let me weigh each check in turn.\n", Some("max_tokens"))),
        Ok((fenced.as_str(), Some("end_turn"))),
    ]);

    let judged = judge_contract(&client, contract(), &expected())
        .await
        .expect("the continued reply is complete");

    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    let calls = client.calls();
    assert_eq!(calls.len(), 2, "one continuation");
    assert_eq!(calls[1].len(), 3, "a continuation of the same reply");
}

/// A first reply whose text is not a JSON prefix cannot be continued into a
/// document: it is asked afresh, never a stalled judge.
#[tokio::test]
async fn a_truncated_first_reply_that_contradicts_json_is_asked_afresh() {
    let client = Scripted::new(vec![
        Ok((r#"{"decisions": ]"#, Some("max_tokens"))),
        Ok((ACCEPTED, Some("end_turn"))),
    ]);

    let judged = judge_contract(&client, contract(), &expected())
        .await
        .expect("the fresh reply is complete");

    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(
        client.calls()[1].len(),
        1,
        "a fresh ask, not a continuation"
    );
}
