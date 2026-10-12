//! Each refused reply's first error falls in one (verdict, kind) class of a
//! closed set; the critic's progress is judged by these classes.

use super::*;

fn class(reply: &str) -> (Option<usize>, RefusalKind) {
    parse_fidelity_reply(reply, &obligations(), &tasks())
        .expect_err("refused")
        .class()
}

const DONE: &str = r#"{"obligation_id":"DONE-9","necessarily_true":true,"supporting_task_id":"TASK-WS-009","supporting_text":"Bindings may carry status: pending-ingest.","reason":"r"}"#;
const AC: &str = r#"{"obligation_id":"AC-WS-003","necessarily_true":true,"supporting_task_id":"TASK-WS-009","supporting_text":"Bindings may carry status: pending-ingest.","reason":"r"}"#;

#[test]
fn shape_errors_map_to_their_verdict_and_kind() {
    assert_eq!(class("not json"), (None, RefusalKind::Syntax));
    assert_eq!(
        class(&format!("{{\"verdicts\":[{AC}]")).1,
        RefusalKind::Truncated
    );
    assert_eq!(
        class(&format!("{{\"verdicts\":[{AC},{DONE}]}} trailing")),
        (None, RefusalKind::Syntax)
    );
    assert_eq!(class("{}"), (None, RefusalKind::MissingField("verdicts")));
    assert_eq!(
        class(r#"{"verdicts":5}"#),
        (None, RefusalKind::InvalidType("verdicts"))
    );
    let stray = |name: &str| {
        format!(
            r#"{{"verdicts":[{AC},{{"obligation_id":"DONE-9","necessarily_true":true,"supporting_task_id":"TASK-WS-009","supporting_text":"Bindings may carry status: pending-ingest.","reason":"r","{name}":""}}]}}"#
        )
    };
    assert_eq!(class(&stray("x")), (Some(1), RefusalKind::UnknownField));
    assert_eq!(
        class(&stray("a_much_longer_name")),
        class(&stray("x")),
        "the name is ignored"
    );
    assert_eq!(
        class(&format!(
            r#"{{"verdicts":[{{"obligation_id":"AC-WS-003","necessarily_true":true}},{DONE}]}}"#
        )),
        (Some(0), RefusalKind::MissingField("reason"))
    );
    assert_eq!(
        class(&format!(
            r#"{{"verdicts":[{{"obligation_id":"AC-WS-003","necessarily_true":true,"supporting_task_id":"TASK-WS-009","supporting_text":"Bindings may carry status: pending-ingest.","reason":5}},{DONE}]}}"#
        )),
        (Some(0), RefusalKind::InvalidType("reason"))
    );
    // An index past the cluster is the document's: indexes stay in the cluster.
    let past = format!(
        r#"{{"verdicts":[{AC},{DONE},{AC},{{"obligation_id":"X","necessarily_true":true,"supporting_task_id":"TASK-WS-009","supporting_text":"Bindings may carry status: pending-ingest.","reason":"r","stray":1}}]}}"#
    );
    assert_eq!(class(&past), (None, RefusalKind::UnknownField));
}

#[test]
fn validation_errors_map_to_their_verdict_and_kind() {
    assert_eq!(
        class(r#"{"verdicts":[]}"#),
        (None, RefusalKind::WrongObligationIds)
    );
    assert_eq!(
        class(&format!(r#"{{"verdicts":[{AC},{AC}]}}"#)),
        (None, RefusalKind::WrongObligationIds)
    );
    // The verdict index is the reply's own array index.
    let false_without = |key_and_value: &str| {
        format!(
            r#"{{"verdicts":[{DONE},{{"obligation_id":"AC-WS-003","necessarily_true":false,"reason":"r"{key_and_value}}}]}}"#
        )
    };
    assert_eq!(
        class(&false_without(r#","quoted_task_text":"temporary""#)),
        (Some(1), RefusalKind::BlankRequired("weakest_task_id"))
    );
    assert_eq!(
        class(&false_without(r#","weakest_task_id":"TASK-WS-005""#)),
        (Some(1), RefusalKind::BlankRequired("quoted_task_text"))
    );
    assert_eq!(
        class(&false_without(
            r#","weakest_task_id":"TASK-WS-005","quoted_task_text":"paraphrase""#
        )),
        (Some(1), RefusalKind::NotVerbatim("quoted_task_text"))
    );
    assert_eq!(
        class(&format!(
            r#"{{"verdicts":[{{"obligation_id":"AC-WS-003","necessarily_true":true,"supporting_task_id":"TASK-WS-009","supporting_text":"Bindings may carry status: pending-ingest.","reason":" "}},{DONE}]}}"#
        )),
        (Some(0), RefusalKind::BlankRequired("reason"))
    );
}

/// #434d: read verdict by verdict, each verdict passes or fails alone, by
/// exactly the checks a whole reply passes.
fn per_verdict(
    reply: &str,
) -> Result<Vec<Result<FidelityVerdict, FidelityRefusal>>, FidelityRefusal> {
    parse_fidelity_verdicts(reply, &obligations(), &tasks())
}

const PARAPHRASED: &str = r#"{"obligation_id":"AC-WS-003","necessarily_true":false,"weakest_task_id":"TASK-WS-005","reason":"r","quoted_task_text":"tests run in a scratch root"}"#;
const LOOPHOLE: &str = r#"{"obligation_id":"AC-WS-003","necessarily_true":false,"weakest_task_id":"TASK-WS-005","reason":"r","quoted_task_text":"the shared project root is untouched."}"#;

#[test]
fn fidelity_per_verdict_keeps_the_verdicts_that_pass_alone() {
    let read = per_verdict(&format!(r#"{{"verdicts":[{PARAPHRASED},{DONE}]}}"#)).expect("document");
    let refusal = read[0].as_ref().expect_err("not verbatim");
    assert_eq!(refusal.kind, RefusalKind::NotVerbatim("quoted_task_text"));
    assert!(refusal.message.contains("AC-WS-003"), "{}", refusal.message);
    // The verdict kept is the one the whole-reply parse gives once the other
    // verdict passes too: the same checks, the same value.
    let whole = parse_fidelity_reply(
        &format!(r#"{{"verdicts":[{LOOPHOLE},{DONE}]}}"#),
        &obligations(),
        &tasks(),
    )
    .expect("whole reply");
    assert_eq!(read[1].as_ref().expect("kept"), &whole[1]);
    assert_eq!(
        parse_fidelity_reply(
            &format!(r#"{{"verdicts":[{PARAPHRASED},{DONE}]}}"#),
            &obligations(),
            &tasks()
        )
        .expect_err("a whole reply is still refused")
        .kind,
        RefusalKind::NotVerbatim("quoted_task_text")
    );

    // A verdict the reply lacks fails alone.
    let read = per_verdict(&format!(r#"{{"verdicts":[{DONE}]}}"#)).expect("document");
    assert_eq!(
        read[0].as_ref().unwrap_err().kind,
        RefusalKind::WrongObligationIds
    );
    assert_eq!(read[1].as_ref().expect("kept"), &whole[1]);
}

#[test]
fn fidelity_per_verdict_takes_nothing_from_a_refused_document() {
    let stray = format!(
        r#"{{"verdicts":[{LOOPHOLE},{{"obligation_id":"DONE-9","necessarily_true":true,"supporting_task_id":"TASK-WS-009","supporting_text":"Bindings may carry status: pending-ingest.","reason":"r","stray":""}}]}}"#
    );
    let extra = r#"{"obligation_id":"OTHER-1","necessarily_true":true,"supporting_task_id":"TASK-WS-009","supporting_text":"Bindings may carry status: pending-ingest.","reason":"r"}"#;
    for (reply, kind) in [
        ("not json".to_string(), RefusalKind::Syntax),
        (stray, RefusalKind::UnknownField),
        (
            format!(r#"{{"verdicts":[{LOOPHOLE},{DONE},{DONE}]}}"#),
            RefusalKind::WrongObligationIds,
        ),
        (
            format!(r#"{{"verdicts":[{LOOPHOLE},{DONE},{extra}]}}"#),
            RefusalKind::WrongObligationIds,
        ),
    ] {
        let refusal = per_verdict(&reply).expect_err("the document is refused");
        assert_eq!(refusal.kind, kind, "{reply}");
        assert_eq!(
            parse_fidelity_reply(&reply, &obligations(), &tasks()).unwrap_err(),
            refusal,
            "the same refusal a whole reply gets"
        );
    }
}
