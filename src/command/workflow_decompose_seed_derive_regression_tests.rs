use super::*;
use serde_json::json;

/// Issue 357 stamps the host criterion before it keeps an entry, so the
/// gate's candidate holds the criterion text while the replies hold the
/// author's. The gate still holds that round: no reply is "since" it.
#[test]
fn a_gate_holds_the_round_whose_replies_it_stamped() {
    let raw = |id: &str| {
        let mut entry = entry(id);
        entry["criterion"] = json!("the author's own words");
        entry.to_string()
    };
    let candidate = json!({"entries": [entry("AC-1"), entry("AC-2")]}).to_string();
    let records = vec![
        reply("acceptance-author-AC-1-4", "01:00:00", &raw("AC-1")),
        reply("acceptance-author-AC-2-4", "01:01:00", &raw("AC-2")),
        gate(
            "freeze-acceptance",
            "02:00:00",
            &candidate,
            Vec::new(),
            false,
        ),
    ];
    let derived = super::super::derive(&records, &[], &criteria(&["AC-1", "AC-2"])).unwrap();
    let (_, _, replies, invalid, carried) = entries_seed(&derived);
    assert!(replies.is_empty(), "{replies:?}");
    assert!(invalid.is_empty(), "{invalid:?}");
    assert_eq!(carried, 2);
}

#[test]
fn acceptance_reply_seed_keeps_the_criterion_from_its_author_prompt() {
    let call_id = "acceptance-author-AC-1-4";
    let mut record = reply(call_id, "01:00:00", &entry("AC-1").to_string());
    record.call.options.task = Some(
        "prompt\nAuthor ONLY entry AC-1: criterion cut from the current PRD\ncontinued criterion\nAll criterion IDs and text (for consistency): {}\ncatalogue".into(),
    );
    let derived = super::super::derive(&[record], &[], &criteria(&["AC-1"])).unwrap();
    let SubjectSeed::Entries { replies, .. } = &derived.subjects["acceptance"] else {
        panic!("expected acceptance seed")
    };
    assert_eq!(
        replies[0].criterion.as_deref(),
        Some("criterion cut from the current PRD\ncontinued criterion")
    );
}

#[test]
fn stale_acceptance_candidate_is_carried_without_verdict_or_findings() {
    let candidate = json!({
        "entries": [entry("AC-1")],
        "supplementary": [entry("SUP-REQ-1"), entry("SUP-REQ-2")]
    })
    .to_string();
    let mut stale = gate(
        "freeze-acceptance",
        "02:00:00",
        &candidate,
        vec![finding("stale refutation", "AC-1")],
        false,
    );
    stale.result.data["logicVersion"] = json!(1);
    let derived = super::super::derive(&[stale], &[], &criteria(&["AC-1"])).unwrap();
    let SubjectSeed::Entries {
        gates,
        candidate,
        carried_entries,
        carried,
        refuted_ids,
        ..
    } = &derived.subjects["acceptance"]
    else {
        panic!("expected acceptance seed")
    };
    assert!(gates.is_empty());
    assert!(candidate.is_none());
    assert!(refuted_ids.is_empty());
    assert_eq!(*carried, 3);
    assert_eq!(
        carried_entries
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["AC-1", "SUP-REQ-1", "SUP-REQ-2"]
    );
}

#[test]
fn later_current_candidate_omission_does_not_erase_earlier_entry() {
    let full = json!({"entries": [entry("AC-1"), entry("AC-2")]}).to_string();
    let omitted = json!({"entries": [entry("AC-1")]}).to_string();
    let mut stale = gate(
        "freeze-acceptance",
        "01:00:00",
        &full,
        vec![finding("old finding", "AC-2")],
        false,
    );
    stale.result.data["logicVersion"] = json!(1);
    let records = [
        stale,
        gate(
            "freeze-acceptance",
            "02:00:00",
            &omitted,
            vec![finding("uncovered requirement", "acceptance")],
            false,
        ),
    ];
    let derived = super::super::derive(&records, &[], &criteria(&["AC-1", "AC-2"])).unwrap();
    let SubjectSeed::Entries {
        carried_entries,
        carried,
        gates,
        refuted_ids,
        ..
    } = &derived.subjects["acceptance"]
    else {
        panic!("expected acceptance seed")
    };
    assert_eq!(*carried, 2);
    assert_eq!(gates.len(), 1, "the stale verdict and finding are excluded");
    assert!(refuted_ids.is_empty());
    assert_eq!(
        carried_entries
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["AC-1", "AC-2"]
    );
}

#[test]
fn explicit_current_refutation_of_an_omitted_held_entry_is_seeded() {
    let candidate = json!({"entries": [entry("AC-1")]}).to_string();
    let records = [gate(
        "freeze-acceptance",
        "02:00:00",
        &candidate,
        vec![finding("check 'AC-2' was refuted by the judge", "AC-2")],
        false,
    )];
    let derived = super::super::derive(&records, &[], &criteria(&["AC-1", "AC-2"])).unwrap();
    let SubjectSeed::Entries {
        refuted_ids,
        carried_entries,
        ..
    } = &derived.subjects["acceptance"]
    else {
        panic!("expected acceptance seed")
    };
    assert_eq!(refuted_ids, &["AC-2"]);
    assert_eq!(carried_entries.len(), 1);
}

#[test]
fn an_accepted_unreadable_reply_names_its_id_and_reconstruction_failure() {
    let records = [reply("acceptance-author-AC-7", "01:00:00", "not JSON")];
    let derived = super::super::derive(&records, &[], &criteria(&["AC"])).unwrap();
    let SubjectSeed::Entries {
        unreadable_replies,
        carried,
        ..
    } = &derived.subjects["acceptance"]
    else {
        panic!("expected acceptance seed")
    };
    assert_eq!(*carried, 0);
    assert!(unreadable_replies["AC"].contains("could not be reconstructed"));
}

#[test]
fn supplementary_host_stamp_uses_current_prd_text_not_finding_text() {
    let current = "full current derived PRD requirement";
    let criteria = criteria(&[]);
    let requirement_texts = [("REQ-1".to_string(), current.to_string())]
        .into_iter()
        .collect();
    let host = super::super::host_owned::HostOwned::new(&criteria, &requirement_texts);
    assert_eq!(
        host.stamp(entry("SUP-REQ-1"), "SUP-REQ-1")["criterion"],
        current
    );
}
