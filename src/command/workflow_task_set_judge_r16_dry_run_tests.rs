//! Read-only replay; recorded domain text and identifiers never become fixtures.
use super::super::continuation_tests::contract;
use super::*;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
#[ignore = "operator supplies R16_RUN_DIR; reads recorded replies without a provider"]
fn r16_judge_screen_dry_run() {
    let root = std::path::PathBuf::from(std::env::var_os("R16_RUN_DIR").expect("R16_RUN_DIR"));
    let events: Vec<Value> = fs::read_to_string(root.join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let mut replies = Vec::new();
    for file in fs::read_dir(root.join("v2/results")).unwrap() {
        let record = read(&file.unwrap().path());
        let call = record["call"]["id"].as_str().unwrap();
        let seq = events
            .iter()
            .find(|e| e["kind"] == "author_attempt_rejected" && e["detail"]["call_id"] == call)
            .and_then(|e| e["seq"].as_u64())
            .unwrap_or(0);
        if let Some(findings) = record
            .pointer("/result/data/gateEnvelope/policy_findings")
            .and_then(Value::as_array)
        {
            for finding in findings {
                let text = finding["text"].as_str().unwrap_or_default();
                if text.contains("was refuted by the host judge; reason:") {
                    replies.push((seq, call.to_owned(), text.to_owned()));
                }
            }
        }
    }
    replies.sort();
    // Full judgments are persisted alongside the workflow, before author-note
    // truncation. Require a unique match in both fields; never replay an envelope.
    let cache = root
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("freeze-cache/judge");
    let mut recorded = Vec::new();
    for file in fs::read_dir(cache).unwrap() {
        let record = read(&file.unwrap().path());
        for field in ["acceptance", "supplementary"] {
            if let Some(entries) = record["judged"][field].as_array() {
                recorded.extend(entries.iter().cloned());
            }
        }
    }
    let mut reasks = 0;
    for (seq, call, text) in &replies {
        let (id, rest) = text
            .strip_prefix("check '")
            .unwrap()
            .split_once("' was refuted by the host judge; reason: ")
            .unwrap();
        let (reason, rest) = rest.split_once("; counterexample: ").unwrap();
        let counterexample = rest
            .strip_suffix("; replace the check with one that fails in that state")
            .unwrap();
        let reason: String = serde_json::from_str(reason).unwrap();
        let counterexample: String = serde_json::from_str(counterexample).unwrap();
        let matches: Vec<_> = recorded
            .iter()
            .filter(|entry| {
                entry["id"] == id
                    && entry["judgment"]["verdict"] == "refuted"
                    && [("reason", &reason), ("counterexample", &counterexample)]
                        .iter()
                        .all(|(field, prefix)| {
                            let full = entry["judgment"][field].as_str().unwrap();
                            if let Some(prefix) = prefix.strip_suffix('…') {
                                full.starts_with(prefix)
                            } else {
                                full == prefix.as_str()
                            }
                        })
            })
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "e{seq}/{id}: full judgment must be unique"
        );
        let reason = matches[0]["judgment"]["reason"].as_str().unwrap();
        let counterexample = matches[0]["judgment"]["counterexample"].as_str().unwrap();
        // R16 predates kind/cited_paths. Only wire fields are adapted; recorded
        // full reason and counterexample are untouched.
        let prose = format!("{reason} {counterexample}").to_lowercase();
        let duplicate = ["duplicat", "relocat", "old copy", "stale copy"]
            .iter()
            .any(|p| prose.contains(p));
        let kind = if toolchain_claim(&prose) && !duplicate {
            "other"
        } else if duplicate {
            "duplicate"
        } else {
            "state"
        };
        let reply = json!({"decisions":[{"id":id,"verdict":"refuted", "kind":kind,
            "cited_paths":[],"reason":reason,"counterexample":counterexample}]})
        .to_string();
        let mut subject = contract();
        subject.acceptance[0].id = id.into();
        let mut screen = Screen::default();
        let missing = screen
            .apply(&mut subject, &reply, &JudgeRoots::default())
            .unwrap();
        let reasked = missing.contains(id);
        let forwarded = if reasked {
            String::new()
        } else {
            author_note(&subject.acceptance[0])
        };
        println!(
            "{id} attempt=e{seq}/{call} decision={} forwarded={}",
            if reasked { "re-ask" } else { "forward" },
            forwarded.chars().take(160).collect::<String>()
        );
        reasks += usize::from(reasked);
    }
    assert_eq!(
        replies.len(),
        17,
        "all recorded refutations must be replayed"
    );
    println!(
        "SUMMARY replies={} reasks={reasks} forwarded={}",
        replies.len(),
        replies.len() - reasks
    );
}
