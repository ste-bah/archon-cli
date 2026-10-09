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

#[test]
#[ignore = "read-only seed dry run; set ARCHON_SEED_DRYRUN_RUN and ARCHON_SEED_DRYRUN_PRD"]
fn resume_seed_dry_run() {
    use archon_workflow::WorkflowV2ResultStore;
    let (Some(root), Some(prd)) = (
        std::env::var_os("ARCHON_SEED_DRYRUN_RUN"),
        std::env::var_os("ARCHON_SEED_DRYRUN_PRD"),
    ) else {
        panic!("dry run requires both ARCHON_SEED_DRYRUN_RUN and ARCHON_SEED_DRYRUN_PRD to be set");
    };
    let root = std::path::PathBuf::from(root);
    let prd = std::path::PathBuf::from(prd);
    let (bytes, _, criteria) = crate::command::workflow_task_set::validate_prd_input(&prd).unwrap();
    let requirement_texts = archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(
        std::str::from_utf8(&bytes).unwrap(),
    );
    let records = WorkflowV2ResultStore::new(root.join("v2"))
        .load_call_records()
        .unwrap();
    let derived =
        super::super::derive_with_requirement_texts(&records, &[], &criteria, &requirement_texts)
            .unwrap();
    let SubjectSeed::Entries {
        carried_entries,
        gates,
        replies,
        refuted_ids,
        invalid,
        ..
    } = &derived.subjects["acceptance"]
    else {
        panic!("no acceptance seed")
    };
    let entry_count = carried_entries
        .iter()
        .filter(|entry| !entry["id"].as_str().unwrap().starts_with("SUP-"))
        .count();
    let supplementary_count = carried_entries.len() - entry_count;
    let reply_criteria: std::collections::BTreeMap<_, _> = replies
        .iter()
        .filter_map(|reply| {
            reply
                .criterion
                .as_ref()
                .map(|criterion| (reply.id.as_str(), criterion.as_str()))
        })
        .collect();
    let reply_ids: std::collections::BTreeSet<_> =
        replies.iter().map(|reply| reply.id.as_str()).collect();
    let missing_prompt_criterion_replies = replies
        .iter()
        .filter(|reply| reply.criterion.is_none())
        .filter(|reply| {
            criteria
                .get(&reply.id)
                .or_else(|| {
                    reply
                        .id
                        .strip_prefix("SUP-")
                        .and_then(|req| requirement_texts.get(req))
                })
                .is_some()
        })
        .count();
    let candidate_fallback_entries = carried_entries
        .iter()
        .filter(|entry| {
            let id = entry["id"].as_str().unwrap();
            !reply_ids.contains(id) && entry["criterion"].as_str().is_some()
        })
        .count();
    let mut reauthor = Vec::new();
    for entry in carried_entries {
        let id = entry["id"].as_str().unwrap();
        let mut reasons = Vec::new();
        if refuted_ids.iter().any(|item| item == id) {
            reasons.push("current gate refutation");
        }
        if invalid.contains_key(id) {
            reasons.push("invalid entry shape");
        }
        let current = criteria.get(id).or_else(|| {
            id.strip_prefix("SUP-")
                .and_then(|req| requirement_texts.get(req))
        });
        if let Some(current) = current {
            match reply_criteria.get(id) {
                Some(prompt_criterion) if *prompt_criterion != current => {
                    reasons.push("#389 author prompt criterion drift");
                }
                Some(_) => {}
                None if reply_ids.contains(id) => {
                    reasons.push("#389 missing author prompt criterion")
                }
                None if entry["criterion"].as_str() != Some(current) => {
                    reasons.push("#381 criterion drift")
                }
                None => {}
            }
        }
        if !reasons.is_empty() {
            reauthor.push(format!("{id}: {}", reasons.join(", ")));
        }
    }
    let mut out = format!(
        "acceptance carried={} entries={} supplementary={} gate_count={} next_call=freeze-acceptance\n",
        carried_entries.len(),
        entry_count,
        supplementary_count,
        gates.len()
    );
    out.push_str(&format!(
        "carried_ids={}\n",
        carried_entries
            .iter()
            .filter_map(|e| e["id"].as_str())
            .collect::<Vec<_>>()
            .join(",")
    ));
    out.push_str(&format!("reauthor_count={}\n", reauthor.len()));
    out.push_str(&format!(
        "missing_prompt_criterion_replies_reauthored={missing_prompt_criterion_replies}\n"
    ));
    out.push_str(&format!(
        "entries_using_candidate_fallback={candidate_fallback_entries}\n"
    ));
    for reason in &reauthor {
        let (id, reasons) = reason.split_once(": ").unwrap();
        let current = criteria.get(id).or_else(|| {
            id.strip_prefix("SUP-")
                .and_then(|req| requirement_texts.get(req))
        });
        let prompt_criterion = current.expect("every re-author id has current PRD text");
        out.push_str(&format!("{id}: reason={reasons}; prompt_criterion={prompt_criterion:?}; equals_current_derived_prd_text=true\n"));
    }
    let reauthored_ids: std::collections::BTreeSet<_> = reauthor
        .iter()
        .filter_map(|item| item.split_once(": ").map(|(id, _)| id))
        .collect();
    let second_reauthor_ids: Vec<_> = carried_entries
        .iter()
        .filter_map(|entry| {
            let id = entry["id"].as_str().unwrap();
            if reauthored_ids.contains(id) {
                return None; // Simulated valid reply is stamped with its prompt criterion.
            }
            let current = criteria.get(id).or_else(|| {
                id.strip_prefix("SUP-")
                    .and_then(|req| requirement_texts.get(req))
            });
            let reply_uses_current_prompt = current.is_some_and(|criterion| {
                reply_criteria
                    .get(id)
                    .is_some_and(|prompt| *prompt == criterion)
            });
            if reply_uses_current_prompt {
                return None;
            }
            (invalid.contains_key(id)
                || refuted_ids.iter().any(|item| item == id)
                || current.is_some_and(|criterion| entry["criterion"].as_str() != Some(criterion)))
            .then_some(id)
        })
        .collect();
    let second_reauthors = second_reauthor_ids.len();
    assert_eq!(
        second_reauthors, 0,
        "the simulated valid prompt replies settle every re-author id: {second_reauthor_ids:?}"
    );
    out.push_str(&format!("second_resume_reauthor_count={second_reauthors} (simulated valid replies use each prompt criterion)\n"));
    if let Some(previous) = std::env::var_os("ARCHON_SEED_DRYRUN_PREVIOUS") {
        let previous = std::fs::read_to_string(previous).unwrap();
        let prior_ids: std::collections::BTreeSet<_> = previous
            .lines()
            .find_map(|line| line.strip_prefix("carried_ids="))
            .unwrap_or_default()
            .split(',')
            .filter(|id| !id.is_empty())
            .collect();
        let current_ids: std::collections::BTreeSet<_> = carried_entries
            .iter()
            .filter_map(|entry| entry["id"].as_str())
            .collect();
        let added: Vec<_> = current_ids.difference(&prior_ids).copied().collect();
        let removed: Vec<_> = prior_ids.difference(&current_ids).copied().collect();
        let prior_reasons: std::collections::BTreeMap<_, _> = previous
            .lines()
            .filter_map(|line| {
                let (id, reason) = line.split_once(": reason=")?;
                Some((
                    id,
                    reason.split_once(';').map_or(reason, |(reason, _)| reason),
                ))
            })
            .collect();
        let current_reasons: std::collections::BTreeMap<_, _> = reauthor
            .iter()
            .filter_map(|item| {
                let (id, reason) = item.split_once(": ")?;
                Some((id, reason))
            })
            .collect();
        let changed_reauthors: Vec<_> = prior_reasons
            .keys()
            .chain(current_reasons.keys())
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| {
                let before = prior_reasons.get(id).copied();
                let after = current_reasons.get(id).copied();
                (before != after).then(|| format!("{id}: {before:?} -> {after:?}"))
            })
            .collect();
        out.push_str(&format!(
            "changed_vs_previous_carried_added={added:?} removed={removed:?}\n"
        ));
        out.push_str(&format!(
            "changed_vs_previous_reauthors={changed_reauthors:?}\n"
        ));
        if !added.is_empty() || !removed.is_empty() || !changed_reauthors.is_empty() {
            out.push_str(
                "change_reason=seed reconstruction from the supplied run records and current PRD\n",
            );
        }
    }
    std::fs::write(root.parent().unwrap().join("dryrun4.txt"), &out).unwrap();
    println!("{out}");
}
