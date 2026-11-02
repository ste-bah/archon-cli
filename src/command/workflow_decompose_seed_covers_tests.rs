use super::*;

fn with_covers_prompt(mut record: WorkflowV2CallRecord, covers: &Value) -> WorkflowV2CallRecord {
    record.call.options.task = Some(format!(
        "Author ONLY entry AC-1: criterion of AC-1\nThis entry currently covers {covers}. Keep every one of them in covers unless the new check no longer fails when that requirement is violated; the host re-opens each one you drop that no other check covers as an owed supplementary check.\nAll criterion IDs and text (for consistency): {{}}"
    ));
    record
}

#[test]
fn refused_reply_is_not_the_entry_a_later_omission_replaces() {
    let mut published = entry("AC-1");
    published["covers"] = json!(["REQ-P"]);
    let mut refused = entry("AC-1");
    refused["covers"] = json!(["REQ-A"]);
    refused["gap_permitted"] = json!("not a bool");
    let mut omitted = entry("AC-1");
    omitted.as_object_mut().unwrap().remove("covers");
    omitted["check"]["command"] = json!("stronger");
    // The live loop never keeps a reply its entry validator refuses.
    assert!(!record::entry_refusals("AC-1", &refused).unwrap().is_empty());
    let records = [
        gate(
            "freeze-acceptance",
            "10:00:00",
            &json!({"entries":[published]}).to_string(),
            vec![finding("check 'AC-1': weak", "AC-1")],
            true,
        ),
        reply("acceptance-author-AC-1-7", "10:01:00", &refused.to_string()),
        with_covers_prompt(
            reply(
                "acceptance-author-AC-1-10",
                "10:02:00",
                &omitted.to_string(),
            ),
            &json!(["REQ-P"]),
        ),
    ];
    let derived = derive(&records, &[], &criteria(&["AC-1"])).unwrap();
    let (_, _, replies, _, _) = entries_seed(&derived);
    assert_eq!(
        reply_entry(&replies[0].text, "AC-1").unwrap()["covers"],
        json!(["REQ-P"])
    );
}

#[test]
fn old_binary_omission_keeps_its_gate_last() {
    let mut first = entry("AC-1");
    first["covers"] = json!(["REQ-A"]);
    let mut omitted = entry("AC-1");
    omitted.as_object_mut().unwrap().remove("covers");
    omitted["check"]["command"] = json!("stronger");
    // An older binary kept the omission as no covers in its candidate.
    let records = [
        reply("acceptance-author-AC-1-4", "10:00:00", &first.to_string()),
        gate(
            "freeze-acceptance",
            "10:01:00",
            &json!({"entries":[first]}).to_string(),
            vec![finding("check 'AC-1': weak", "AC-1")],
            true,
        ),
        reply("acceptance-author-AC-1-7", "10:02:00", &omitted.to_string()),
        gate(
            "freeze-acceptance",
            "10:03:00",
            &json!({"entries":[omitted]}).to_string(),
            vec![finding("check 'AC-1': second", "AC-1")],
            true,
        ),
    ];
    let derived = derive(&records, &[], &criteria(&["AC-1"])).unwrap();
    let (gates, _, replies, _, _) = entries_seed(&derived);
    assert_eq!(
        gates.last().unwrap().call_id,
        "freeze-acceptance-10:03:00",
        "{gates:?}"
    );
    assert!(replies.is_empty(), "{replies:?}");
}

#[test]
fn omitted_reply_inherits_exactly_its_own_prompt_array() {
    // A changed base and a re-author-all round cannot use the stale candidate.
    let mut stale = entry("AC-1");
    stale["covers"] = json!(["REQ-STALE"]);
    let expected = json!([
        "REQ-NEW",
        "quoted \"id\"",
        "right ] bracket",
        "line\nbreak",
        "back\\slash"
    ]);
    let mut omitted = entry("AC-1");
    omitted.as_object_mut().unwrap().remove("covers");
    let records = [
        gate(
            "freeze-acceptance",
            "10:00:00",
            &json!({"entries":[stale]}).to_string(),
            vec![],
            true,
        ),
        with_covers_prompt(
            reply("acceptance-author-AC-1-7", "10:01:00", &omitted.to_string()),
            &expected,
        ),
    ];
    let derived = derive(&records, &[], &criteria(&["AC-1"])).unwrap();
    let (_, _, replies, invalid, _) = entries_seed(&derived);
    assert!(invalid.is_empty(), "{invalid:?}");
    assert_eq!(
        reply_entry(&replies[0].text, "AC-1").unwrap()["covers"],
        expected
    );
    assert_eq!(replies[0].criterion.as_deref(), Some("criterion of AC-1"));
}

#[test]
fn old_binary_reply_without_prompt_line_is_unchanged() {
    let mut previous = entry("AC-1");
    previous["covers"] = json!(["REQ-OLD"]);
    let mut omitted = entry("AC-1");
    omitted.as_object_mut().unwrap().remove("covers");
    let text = omitted.to_string();
    let records = [
        reply(
            "acceptance-author-AC-1-4",
            "10:00:00",
            &previous.to_string(),
        ),
        reply("acceptance-author-AC-1-7", "10:01:00", &text),
    ];
    let derived = derive(&records, &[], &criteria(&["AC-1"])).unwrap();
    let (_, _, replies, _, _) = entries_seed(&derived);
    assert_eq!(replies[0].text, text);
    assert!(
        reply_entry(&replies[0].text, "AC-1")
            .unwrap()
            .get("covers")
            .is_none()
    );
}

#[test]
fn explicit_covers_and_supplementary_host_fields_override_prompt() {
    for covers in [json!([]), json!(["REQ-EXPLICIT"]), Value::Null] {
        let mut explicit = entry("AC-1");
        explicit["covers"] = covers.clone();
        let records = [with_covers_prompt(
            reply(
                "acceptance-author-AC-1-7",
                "10:00:00",
                &explicit.to_string(),
            ),
            &json!(["REQ-PROMPT"]),
        )];
        let derived = derive(&records, &[], &criteria(&["AC-1"])).unwrap();
        let (_, _, replies, _, _) = entries_seed(&derived);
        assert_eq!(
            reply_entry(&replies[0].text, "AC-1").unwrap()["covers"],
            covers
        );
    }
    let mut omitted = entry("SUP-REQ-X");
    omitted.as_object_mut().unwrap().remove("covers");
    let records = [with_covers_prompt(
        reply(
            "acceptance-author-SUP-REQ-X-7",
            "10:00:00",
            &omitted.to_string(),
        ),
        &json!(["REQ-OTHER"]),
    )];
    let derived = super::super::super::derive_with_reply_markers(
        &records,
        &[],
        &BTreeMap::new(),
        &[("REQ-X".to_string(), "requirement".to_string())]
            .into_iter()
            .collect(),
        &[],
    )
    .unwrap();
    let SubjectSeed::Entries {
        carried_entries, ..
    } = &derived.subjects["acceptance"]
    else {
        panic!("expected entries")
    };
    assert_eq!(carried_entries[0]["covers"], json!(["REQ-X"]));
}

#[test]
fn an_earlier_quoted_coverage_line_never_replaces_the_live_one() {
    let mut omitted = entry("AC-1");
    omitted.as_object_mut().unwrap().remove("covers");
    let mut record = with_covers_prompt(
        reply("acceptance-author-AC-1-7", "10:00:00", &omitted.to_string()),
        &json!(["REQ-LIVE"]),
    );
    let task = record.call.options.task.take().unwrap();
    record.call.options.task = Some(format!(
        "Repair these exact authoritative findings:\n- quoted\nThis entry currently covers [\"REQ-DECOY\"].\n{task}"
    ));
    let derived = derive(&[record], &[], &criteria(&["AC-1"])).unwrap();
    let (_, _, replies, _, _) = entries_seed(&derived);
    assert_eq!(
        reply_entry(&replies[0].text, "AC-1").unwrap()["covers"],
        json!(["REQ-LIVE"])
    );
}
