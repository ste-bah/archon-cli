//! Each refused reply's first error falls in one (verdict, kind) class of a
//! closed set; the critic's progress is judged by these classes.

use super::*;

fn class(reply: &str) -> (Option<usize>, RefusalKind) {
    parse_fidelity_reply(reply, &obligations(), &tasks())
        .expect_err("refused")
        .class()
}

const DONE: &str = r#"{"obligation_id":"DONE-9","necessarily_true":true,"reason":"r"}"#;
const AC: &str = r#"{"obligation_id":"AC-WS-003","necessarily_true":true,"reason":"r"}"#;

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
            r#"{{"verdicts":[{AC},{{"obligation_id":"DONE-9","necessarily_true":true,"reason":"r","{name}":""}}]}}"#
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
            r#"{{"verdicts":[{{"obligation_id":"AC-WS-003","necessarily_true":true,"reason":5}},{DONE}]}}"#
        )),
        (Some(0), RefusalKind::InvalidType("reason"))
    );
    // An index past the cluster is the document's: indexes stay in the cluster.
    let past = format!(
        r#"{{"verdicts":[{AC},{DONE},{AC},{{"obligation_id":"X","necessarily_true":true,"reason":"r","stray":1}}]}}"#
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
        (Some(1), RefusalKind::Other)
    );
    assert_eq!(
        class(&format!(
            r#"{{"verdicts":[{{"obligation_id":"AC-WS-003","necessarily_true":true,"reason":" "}},{DONE}]}}"#
        )),
        (Some(0), RefusalKind::BlankRequired("reason"))
    );
}
