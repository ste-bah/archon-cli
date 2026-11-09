//! Host-authored recording stimuli, observed with real processes.
use super::super::super::tests::{Fixture, git};
use super::*;
use archon_workflow::base_snapshot::SnapshotKind;
pub(super) const CHECK: &str = r#"observed=$(sh v*); if test "$observed" = load-failure; then . "$observed"; fi; if test "$observed" != good; then printf 'wrong value: expected good, got %s\n' "$observed"; exit 1; fi"#;
pub(super) async fn messages(
    f: &Fixture,
    id: &str,
    text: &str,
) -> Vec<(&'static str, &'static str, String, serde_json::Value)> {
    let mut messages = Vec::new();
    let mutants: Vec<_> = ["bad\n", "load-failure\n", ""]
        .into_iter()
        .enumerate()
        .map(|(i, after)| SuppliedMutant {
            task_id: "task".into(),
            requirement_id: id.into(),
            kind: if i == 2 {
                MutantKind::BodyGut
            } else {
                MutantKind::Behaviour
            },
            patch: rule::edit_patch(
                "value",
                std::str::from_utf8(super::super::tests::VALUE).unwrap(),
                &if i == 2 {
                    "value() { :; }\nvalue\n".to_string()
                } else {
                    std::str::from_utf8(super::super::tests::VALUE)
                        .unwrap()
                        .replace("good", after.trim())
                },
            ),
        })
        .collect();
    let set = MutantSet {
        schema: BREAKAGE_SCHEMA,
        base_commit: f.lock.baseline_commit.clone().unwrap(),
        mutants: mutants.clone(),
    };
    let probe = f.probe(false);
    let report = prove_breakage(
        &probe,
        BreakageRequest {
            contract: &f.contract,
            chain_digest: &f.lock.digest,
            lock: &f.lock,
            classification: &f.classes,
            mutants: &set,
            max_runs: 10,
            min_deadline_secs: 2,
        },
    )
    .await;
    for (name, mutant) in ["relevant"].into_iter().zip(&mutants) {
        let run = report
            .proofs
            .runs
            .values()
            .find(|r| r.mutant_id == mutant.mutant_id())
            .expect("fixture observation");
        let (patch, output) = super::super::super::store::evidence_text(&report, run).unwrap();
        messages.push((name,"breaker_relevance",judges::relevance_prompt(id,text,"keeps the good value",&patch,output.as_deref().unwrap_or("")),serde_json::json!({"stimulus":"host-authored fixture mutant","observation_outcome":run.outcome,"counted":run.counted})));
    }
    // This failing check exercises an unrelated file; the required value is unchanged.
    let unrelated = f.source.path().join("unrelated");
    std::fs::write(&unrelated, b"0\n").unwrap();
    let observe = || {
        archon_shell::spawn::command("sh").args(["-c", "if test \"$(cat unrelated)\" = 0; then exit 0; fi; printf 'unrelated: expected 0\\n' >&2; exit 1"]).current_dir(f.source.path()).output().unwrap()
    };
    let original = observe();
    assert_eq!(original.status.code(), Some(0));
    std::fs::write(&unrelated, b"1\n").unwrap();
    let changed = observe();
    assert_eq!(changed.status.code(), Some(1));
    let patch = rule::edit_patch("unrelated", "0\n", "1\n");
    let output = String::from_utf8(changed.stderr).unwrap();
    messages.push(("not_relevant","breaker_relevance",judges::relevance_prompt(id,text,"keeps the good value",&patch,&output),serde_json::json!({"stimulus":"host-authored unrelated-file check; required value unchanged","original_exit":original.status.code(),"changed_exit":changed.status.code(),"changed_stderr":output})));
    std::fs::write(unrelated, b"0\n").unwrap();
    // The equivalence examples use an actual constant-output unit. Both
    // variants exit successfully; one changes its observable output.
    std::fs::write(f.source.path().join("unit"), b"printf good\n").unwrap();
    git(f.source.path(), &["add", "unit"]);
    git(
        f.source.path(),
        &["commit", "-qm", "recording fixture unit"],
    );
    let tree = RepositoryTree::load(&RepositoryRecordV1 {
        schema_version: 1,
        repository_root: f.source.path().display().to_string(),
        base_commit: git(f.source.path(), &["rev-parse", "HEAD"]),
        decomposition_run_id: "corpus".into(),
        recorded_at: "corpus".into(),
    })
    .unwrap();
    let snapshot = snapshot_deliverables(&tree, f.source.path(), ["unit"]).unwrap();
    let view = materialise_view(
        f.parent.path(),
        f.source.path(),
        &snapshot,
        &[("unit".into(), SnapshotKind::Blob)],
        &[],
    )
    .unwrap();
    let before = std::str::from_utf8(&view.files["unit"].bytes).unwrap();
    let original = archon_shell::spawn::command("sh")
        .arg("-c")
        .arg(before)
        .output()
        .unwrap();
    assert!(original.status.success());
    for (name, role, after) in [
        ("equivalent", "breaker_equivalence", "printf '%s' good\n"),
        (
            "not_equivalent",
            "breaker_equivalence_second",
            "printf bad\n",
        ),
    ] {
        let changed = archon_shell::spawn::command("sh")
            .arg("-c")
            .arg(after)
            .output()
            .unwrap();
        assert!(changed.status.success());
        assert_eq!(original.stdout == changed.stdout, name == "equivalent");
        let patch = rule::edit_patch("unit", before, after);
        let run = MutantRun {
            check_id: String::new(),
            check_digest: String::new(),
            control_key: String::new(),
            mutant_id: String::new(),
            patch_digest: String::new(),
            task_id: "task".into(),
            requirement_id: id.into(),
            kind: MutantKind::Behaviour,
            touched: vec!["unit".into()],
            outcome: RunOutcome::Survived { elapsed_ms: 0 },
            counted: false,
            not_counted: None,
            evidence: Evidence {
                patch: patch.clone(),
                output: None,
            },
        };
        messages.push((name,role,judges::equivalence_prompt(id,text,&view,&run,&patch),serde_json::json!({"stimulus":"host-authored constant-output fixture unit","original_exit":original.status.code(),"changed_exit":changed.status.code(),"original_stdout":String::from_utf8_lossy(&original.stdout),"changed_stdout":String::from_utf8_lossy(&changed.stdout)})));
    }
    messages
}

#[tokio::test]
async fn recording_stimuli_use_observed_process_results() {
    let f = super::super::tests::fixture(CHECK);
    let messages = messages(
        &f,
        "REQ-VALUE-001",
        "It keeps the good value for every input.",
    )
    .await;
    assert_eq!(messages.len(), 4);
    assert!(
        messages[0]
            .2
            .contains("wrong value: expected good, got bad")
    );
    assert_eq!(messages[0].3["counted"], true);
    assert!(messages[1].2.contains("--- a/unrelated"));
    assert_eq!(messages[1].3["original_exit"], 0);
    assert_eq!(messages[1].3["changed_exit"], 1);
    assert_eq!(
        std::fs::read(f.source.path().join("value")).unwrap(),
        super::super::tests::VALUE
    );
    assert_eq!(messages[2].3["original_stdout"], "good");
    assert_eq!(messages[2].3["changed_stdout"], "good");
    assert_eq!(messages[3].3["original_stdout"], "good");
    assert_eq!(messages[3].3["changed_stdout"], "bad");
}

pub(super) async fn record_pair(
    dir: &Path,
    name: &str,
    client: &dyn WorkflowLlmClient,
    model: &str,
    id: &str,
    text: &str,
    view: &BaseView,
    sources: &mut Vec<serde_json::Value>,
    refused: &mut Vec<String>,
) {
    let files: std::collections::BTreeMap<_, _> = view
        .files
        .iter()
        .map(|(k, f)| (k, String::from_utf8(f.bytes.clone()).unwrap()))
        .collect();
    std::fs::write(
        dir.join(format!("{name}.input.json")),
        serde_json::to_vec_pretty(&serde_json::json!({"id":id,"text":text,"files":files})).unwrap(),
    )
    .unwrap();
    let mut feedback = String::new();
    for attempt in 1..=3 {
        let message = prompt::prompt(id, text, view, &feedback);
        let reply = client
            .run_agent(call::agent_call(view, &message, model, attempt, 120))
            .await
            .unwrap();
        let trial = format!("{name}_round5_attempt{attempt}");
        super::save(dir, &trial, client, model, &message, &reply, sources);
        std::fs::write(dir.join(format!("{name}.txt")), &reply.content).unwrap();
        let parsed = prompt::parse(&reply.content, "task", id, text, view);
        let behaviours = parsed
            .as_ref()
            .map(|p| {
                p.mutants
                    .iter()
                    .filter(|(m, _)| m.kind == MutantKind::Behaviour)
                    .count()
            })
            .unwrap_or(0);
        super::record_parse(dir, sources, &parsed);
        let mut guts = 0;
        for key in [serde_json::to_string(&view.files.keys().collect::<Vec<_>>()).unwrap()] {
            let mut gut_feedback = String::new();
            for retry in 1..=2 {
                let message = prompt::gut_prompt(text, view, &key, &gut_feedback);
                let reply = client
                    .run_agent(call::agent_call(view, &message, model, retry, 120))
                    .await
                    .unwrap();
                let gut_name = format!("{trial}_gut{retry}");
                super::save(dir, &gut_name, client, model, &message, &reply, sources);
                std::fs::write(dir.join(format!("{name}_gut.txt")), &reply.content).unwrap();
                let parsed_gut = prompt::parse_gut(&reply.content, "task", id, text, view, &key);
                super::record_parse(dir, sources, &parsed_gut);
                match parsed_gut {
                    Ok(p) => {
                        guts += p.mutants.len();
                        break;
                    }
                    Err(e) => {
                        gut_feedback = format!("Previous reply failed: {e}");
                        refused.push(format!("{gut_name}: {e}"));
                    }
                }
            }
        }
        eprintln!("{trial}: valid_behaviour={behaviours} valid_body_gut={guts}");
        if behaviours > 0 && guts > 0 {
            return;
        }
        feedback = match parsed {
            Ok(p) => p.refusals.join("; "),
            Err(e) => e,
        };
        refused.push(format!(
            "{trial}: valid_behaviour={behaviours} valid_body_gut={guts}; {feedback}"
        ));
    }
    eprintln!("{name}: stopping after three attempts; success bar not met");
}

pub(super) fn review_recorded(
    dir: &Path,
    f: &Fixture,
    sources: &mut [serde_json::Value],
    refused: &mut Vec<String>,
) -> bool {
    let mut counts = std::collections::BTreeMap::<String, (usize, usize)>::new();
    for source in sources {
        let name = source["case"].as_str().unwrap().to_string();
        if !name.starts_with("breaker") {
            continue;
        }
        let base = if name.starts_with("breaker_r18") {
            "breaker_r18"
        } else {
            "breaker"
        };
        let input: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join(format!("{base}.input.json"))).unwrap())
                .unwrap();
        let mut view = materialise_view(
            f.parent.path(),
            f.source.path(),
            f.snapshot(),
            &[("value".into(), SnapshotKind::Blob)],
            &[],
        )
        .unwrap();
        view.files = input["files"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    super::super::super::super::breaker_view::ViewFile {
                        object: String::new(),
                        bytes: v.as_str().unwrap().as_bytes().to_vec(),
                    },
                )
            })
            .collect();
        let raw = std::fs::read_to_string(dir.join(format!("{name}.txt"))).unwrap();
        let id = input["id"].as_str().unwrap();
        let text = input["text"].as_str().unwrap();
        let parsed = if name.contains("_gut") {
            prompt::parse_gut(
                &raw,
                "task",
                id,
                text,
                &view,
                view.files.keys().next().unwrap(),
            )
        } else {
            prompt::parse(&raw, "task", id, text, &view)
        };
        source["parser_refusals"] = match &parsed {
            Ok(p) => serde_json::json!(p.refusals),
            Err(e) => serde_json::json!([e]),
        };
        let (b, g) = match parsed {
            Ok(p) => {
                refused.extend(p.refusals.into_iter().map(|e| format!("{name}: {e}")));
                (
                    p.mutants
                        .iter()
                        .filter(|(m, _)| m.kind == MutantKind::Behaviour)
                        .count(),
                    p.mutants
                        .iter()
                        .filter(|(m, _)| m.kind == MutantKind::BodyGut)
                        .count(),
                )
            }
            Err(e) => {
                refused.push(format!("{name}: {e}"));
                (0, 0)
            }
        };
        source["valid_behaviour"] = b.into();
        source["valid_body_gut"] = g.into();
        if name.contains("_round5_attempt") {
            let entry = counts.entry(base.into()).or_default();
            entry.0 = entry.0.max(b);
            entry.1 = entry.1.max(g);
        }
    }
    counts.len() == 2 && counts.values().all(|(b, g)| *b > 0 && *g > 0)
}
