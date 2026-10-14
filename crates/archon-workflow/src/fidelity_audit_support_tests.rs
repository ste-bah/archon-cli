//! #434: a true verdict names its supporting task and quotes it verbatim;
//! the quote is checked exactly as a loophole quote is, and it must be an
//! anchor a reader can check. It is evidence, never the reuse condition.

use super::*;

const DONE: &str = r#"{"obligation_id":"DONE-9","necessarily_true":true,"supporting_task_id":"TASK-WS-009","supporting_text":"Bindings may carry status: pending-ingest.","reason":"r"}"#;

fn with_ac(ac: &str) -> String {
    format!(r#"{{"verdicts":[{ac},{DONE}]}}"#)
}

fn true_ac(fields: &str) -> String {
    with_ac(&format!(
        r#"{{"obligation_id":"AC-WS-003","necessarily_true":true,"reason":"r"{fields}}}"#
    ))
}

#[test]
fn a_true_verdict_without_its_supporting_task_or_quote_is_refused_naming_the_key() {
    let cases = [
        (
            r#","supporting_text":"library-level focused tests against a temporary""#,
            "supporting_task_id",
        ),
        (
            r#","supporting_task_id":"","supporting_text":"library-level focused tests against a temporary""#,
            "supporting_task_id",
        ),
        (r#","supporting_task_id":"TASK-WS-005""#, "supporting_text"),
        (
            r#","supporting_task_id":"TASK-WS-005","supporting_text":"  ""#,
            "supporting_text",
        ),
    ];
    for (fields, key) in cases {
        let refusal = parse_fidelity_reply(&true_ac(fields), &obligations(), &tasks())
            .expect_err("a true verdict needs its supporting task and quote");
        assert_eq!(
            refusal.class(),
            (Some(0), RefusalKind::BlankRequired(key)),
            "{fields}"
        );
        assert!(
            refusal.message.contains(&format!("`{key}`")) && refusal.message.contains("AC-WS-003"),
            "{}",
            refusal.message
        );
    }
}

/// An unverifiable quote is never accepted: a paraphrase, or a quote from a
/// task outside the cluster, is refused, and the critic is asked again.
#[test]
fn a_true_verdict_whose_quote_is_not_verbatim_in_its_task_is_refused() {
    let paraphrase = true_ac(
        r#","supporting_task_id":"TASK-WS-005","supporting_text":"focused tests against the shared root""#,
    );
    let refusal =
        parse_fidelity_reply(&paraphrase, &obligations(), &tasks()).expect_err("paraphrase");
    assert_eq!(
        refusal.class(),
        (Some(0), RefusalKind::NotVerbatim("supporting_text"))
    );
    assert!(
        refusal
            .message
            .contains("does not appear verbatim in task 'TASK-WS-005'"),
        "{}",
        refusal.message
    );
    // Verbatim, but in the other task than the one it names.
    let wrong_task = true_ac(
        r#","supporting_task_id":"TASK-WS-005","supporting_text":"Bindings may carry status: pending-ingest.""#,
    );
    assert_eq!(
        parse_fidelity_reply(&wrong_task, &obligations(), &tasks())
            .expect_err("quote of another task")
            .class(),
        (Some(0), RefusalKind::NotVerbatim("supporting_text"))
    );
    let outside = true_ac(
        r#","supporting_task_id":"TASK-WS-404","supporting_text":"library-level focused tests against a temporary""#,
    );
    let error = parse_fidelity_response(&outside, &obligations(), &tasks()).expect_err("outside");
    assert!(error.contains("not in the audited cluster"), "{error}");
}

#[test]
fn a_re_wrapped_or_over_long_supporting_quote_is_kept_when_verbatim() {
    let wrapped = true_ac(
        r#","supporting_task_id":"TASK-WS-005","supporting_text":"against a temporary target root; the shared""#,
    );
    let verdicts = parse_fidelity_response(&wrapped, &obligations(), &tasks()).expect("wrapped");
    assert_eq!(verdicts[0].supporting_task_id, "TASK-WS-005");
    let body = (0..80)
        .map(|n| format!("clause {n:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    let obligations = vec![ClaimedObligation {
        id: "OBL-1".into(),
        text: "The result exists.".into(),
    }];
    let tasks = vec![ClaimingTask {
        task_id: "TASK-A-001".into(),
        text: format!("# TASK-A-001\n\n{body}\n"),
    }];
    let long: String = body.chars().take(MAX_QUOTE_CHARS + 50).collect();
    let reply = serde_json::json!({"verdicts": [{"obligation_id": "OBL-1", "necessarily_true": true, "reason": "r", "supporting_task_id": "TASK-A-001", "supporting_text": long}]});
    let verdicts =
        parse_fidelity_response(&reply.to_string(), &obligations, &tasks).expect("cut, verbatim");
    assert_eq!(verdicts[0].supporting_text.chars().count(), MAX_QUOTE_CHARS);
    assert!(
        verdict_holds(&verdicts[0], &tasks),
        "a cut verdict holds as stored"
    );
}

/// `verdict_holds` re-runs a fresh reply's check on a stored verdict. It is
/// the re-validation of a record, never the reuse condition: the store
/// reuses a verdict only over byte-identical task texts (#434), so these
/// cases only show which edits the check itself notices.
#[test]
fn verdict_holds_re_checks_a_stored_verdict_against_the_given_texts() {
    let reply = r#"{"verdicts":[
        {"obligation_id":"AC-WS-003","necessarily_true":false,"weakest_task_id":"TASK-WS-005","reason":"r","quoted_task_text":"the shared project root is untouched"},
        {"obligation_id":"DONE-9","necessarily_true":true,"supporting_task_id":"TASK-WS-009","supporting_text":"Bindings may carry status: pending-ingest.","reason":"r"}
    ]}"#;
    let verdicts = parse_fidelity_response(reply, &obligations(), &tasks()).expect("parses");
    assert!(
        verdicts
            .iter()
            .all(|verdict| verdict_holds(verdict, &tasks()))
    );
    let mut removed = tasks();
    removed[1].text = "# TASK-WS-009\n\nBindings are always complete.\n".into();
    assert!(
        !verdict_holds(&verdicts[1], &removed),
        "supporting quote removed"
    );
    assert!(
        !verdict_holds(&verdicts[1], &tasks()[..1]),
        "supporting task left the cluster"
    );
    let mut doubled = tasks();
    doubled[1]
        .text
        .push_str("Bindings may carry status: pending-ingest.\n");
    assert!(
        !verdict_holds(&verdicts[1], &doubled),
        "quote no longer unique"
    );
}

/// The least anchor a true verdict may rest its quote on: long enough, once
/// in the task, and in its prose. Each failure is refused with an error that
/// says which rule, and the critic is asked again.
#[test]
fn a_supporting_quote_that_is_no_anchor_is_refused_with_the_rule_it_breaks() {
    let text = "# Ingest the widgets into the shared registry now\n\nThe writer must store every widget in the registry it opens.\nShort clause.\nShort clause.\n\n```sh\nthe fenced command stores every widget in the registry\n```\n\n<!-- the hidden note stores every widget in the registry -->\n";
    let obligations = vec![ClaimedObligation {
        id: "OBL-1".into(),
        text: "The registry holds every widget.".into(),
    }];
    let tasks = vec![ClaimingTask {
        task_id: "TASK-A-001".into(),
        text: text.into(),
    }];
    let ask = |quote: &str| {
        let reply = serde_json::json!({"verdicts": [{"obligation_id": "OBL-1", "necessarily_true": true, "reason": "r", "supporting_task_id": "TASK-A-001", "supporting_text": quote}]});
        parse_fidelity_reply(&reply.to_string(), &obligations, &tasks)
    };
    assert!(ask("The writer must store every widget in the registry it opens.").is_ok());
    for (quote, rule) in [
        (
            "Short clause.",
            "it needs at least 8 words or 40 characters",
        ),
        ("Short clause. Short clause.", "it needs at least 8 words"),
        (
            "store every widget in the registry",
            "it needs at least 8 words",
        ),
        (
            "Ingest the widgets into the shared registry now",
            "lies in or across a heading line",
        ),
        (
            "the fenced command stores every widget in the registry",
            "lies in or across a code fence",
        ),
        (
            "the hidden note stores every widget in the registry",
            "lies in or across an HTML comment",
        ),
        (
            "registry now The writer must store every widget",
            "lies in or across a heading line",
        ),
    ] {
        let refusal = ask(quote).expect_err(quote);
        assert_eq!(
            refusal.class(),
            (Some(0), RefusalKind::WeakAnchor),
            "{quote}"
        );
        assert!(
            refusal.message.contains(rule),
            "{quote}: {}",
            refusal.message
        );
    }
    let twice = "The writer must store every widget in the registry it opens.";
    let mut repeated = tasks.clone();
    repeated[0].text.push_str(twice);
    let reply = serde_json::json!({"verdicts": [{"obligation_id": "OBL-1", "necessarily_true": true, "reason": "r", "supporting_task_id": "TASK-A-001", "supporting_text": twice}]});
    let refusal =
        parse_fidelity_reply(&reply.to_string(), &obligations, &repeated).expect_err("found twice");
    assert!(
        refusal.message.contains("appears 2 times"),
        "{}",
        refusal.message
    );
}

#[test]
fn a_contradiction_finding_quotes_both_verdicts() {
    let refuted = FidelityVerdict {
        obligation_id: "AC-WS-003".into(),
        necessarily_true: false,
        weakest_task_id: "TASK-WS-005".into(),
        reason: "the root is temporary".into(),
        quoted_task_text: "the shared project root is untouched".into(),
        weakening_quotes: Vec::new(),
        supporting_task_id: String::new(),
        supporting_text: String::new(),
    };
    let supported = FidelityVerdict {
        necessarily_true: true,
        weakest_task_id: String::new(),
        reason: "the bindings are required".into(),
        quoted_task_text: String::new(),
        supporting_task_id: "TASK-WS-009".into(),
        supporting_text: "Bindings may carry status: pending-ingest.".into(),
        ..refuted.clone()
    };
    assert_eq!(
        fidelity_contradiction_finding(&refuted, &supported, &["TASK-WS-005".to_string()]),
        "obligation AC-WS-003 is claimed by TASK-WS-005 and the critic's verdicts contradict each other on unchanged evidence; neither is taken — false: the root is temporary — task TASK-WS-005: \"the shared project root is untouched\" — true: the bindings are required — task TASK-WS-009: \"Bindings may carry status: pending-ingest.\""
    );
}

/// The prompt asks a true verdict for its supporting task and quote.
#[test]
fn the_prompt_asks_a_true_verdict_for_its_verbatim_support() {
    let prompt = fidelity_prompt(&obligations(), &tasks(), &SkeletonSummary::absent());
    assert!(prompt.contains("When necessarily_true is true, weakening_quotes is an empty list and supporting_task_id names the listed task"));
    assert!(prompt.contains(&format!(
        "at least {MIN_SUPPORT_WORDS} words of the task's prose that oblige the result and occur only once in that task, never a heading, a code block or an HTML comment"
    )));
}
