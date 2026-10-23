//! Whether an incomplete judge is the judge's own reply (retried with the
//! affected checks) or a provider or host fault (operational) is typed at
//! each producer, never read back from the message. Every error here comes
//! from the real reply and round producers, driven by scripted replies.

use std::collections::BTreeSet;

use super::super::continuation_tests::{HEAD, Scripted, contract};
use super::super::{JUDGE_NO_PROGRESS_WINDOW, judge_contract};
use super::*;

/// The incomplete judge the real producers end with for `replies`.
pub(crate) async fn produced(
    replies: Vec<Result<(&str, Option<&'static str>), &str>>,
) -> anyhow::Error {
    let client = Scripted::new(replies);
    let expected = BTreeSet::from(["AC-X-001".to_string()]);
    let error = judge_contract(&client, contract(), &expected)
        .await
        .expect_err("the scripted judge never completes");
    assert!(JudgeIncomplete::caused(&error).is_some(), "{error:#}");
    error
}

fn incomplete(error: &anyhow::Error) -> &JudgeIncomplete {
    JudgeIncomplete::caused(error).expect("typed incomplete judge")
}

fn repeated(
    reply: Result<(&'static str, Option<&'static str>), &'static str>,
) -> Vec<Result<(&'static str, Option<&'static str>), &'static str>> {
    vec![reply; JUDGE_NO_PROGRESS_WINDOW + 1]
}

#[tokio::test]
async fn an_exhausted_loop_of_unsupported_stop_reasons_is_a_provider_fault() {
    let error = produced(repeated(Ok(("{}", Some("lengthy"))))).await;
    let incomplete = incomplete(&error);
    assert!(
        incomplete.to_string().contains("replies; the last"),
        "{error:#}"
    );
    assert!(
        incomplete.to_string().contains("unsupported stop reason"),
        "{error:#}"
    );
    assert!(!incomplete.is_authored_reply(), "{error:#}");
}

#[tokio::test]
async fn an_exhausted_loop_without_a_finish_reason_is_a_provider_fault() {
    let error = produced(repeated(Ok(("{}", None)))).await;
    let incomplete = incomplete(&error);
    assert!(
        incomplete.to_string().contains("no finish reason"),
        "{error:#}"
    );
    assert!(!incomplete.is_authored_reply(), "{error:#}");
}

#[tokio::test]
async fn a_provider_error_is_a_provider_fault() {
    let error = produced(vec![Err("upstream unavailable")]).await;
    assert!(!incomplete(&error).is_authored_reply(), "{error:#}");
}

#[tokio::test]
async fn an_exhausted_loop_of_unparsable_replies_is_the_judges_own() {
    let error = produced(repeated(Ok(("no json", Some("end_turn"))))).await;
    let incomplete = incomplete(&error);
    assert!(
        incomplete.to_string().contains("replies; the last"),
        "{error:#}"
    );
    assert!(incomplete.is_authored_reply(), "{error:#}");
}

#[tokio::test]
async fn a_continuation_that_adds_nothing_is_the_judges_own() {
    let error = produced(vec![
        Ok((HEAD, Some("max_tokens"))),
        Ok(("", Some("length"))),
    ])
    .await;
    let incomplete = incomplete(&error);
    assert!(incomplete.to_string().contains("no token"), "{error:#}");
    assert!(incomplete.is_authored_reply(), "{error:#}");
}

/// D2: a judge that refuses to judge what it is shown has given its own
/// outcome about those checks; it is never the provider's fault.
#[tokio::test]
async fn an_exhausted_loop_of_refusals_is_the_judges_own() {
    let error = produced(repeated(Ok(("", Some("refusal"))))).await;
    let incomplete = incomplete(&error);
    assert!(
        incomplete.to_string().contains("refused to judge"),
        "{error:#}"
    );
    assert!(incomplete.is_authored_reply(), "{error:#}");
}
