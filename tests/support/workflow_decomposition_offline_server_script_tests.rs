use super::{fidelity_answer, scripted_answer};

#[test]
fn routes_current_acceptance_author_prompt_and_extracts_its_id() {
    let prompt = r#"{"task":"Author exactly one acceptance entry identified below. Author ONLY entry AC-X-001: The repository records a JSON outcome artifact at `.archon/proof/outcome.json` with the boolean field `ready` set to true."}"#;
    let (role, answer) = scripted_answer(prompt);
    assert_eq!(role, "acceptance");
    assert!(answer.contains(r#""id":"AC-X-001""#), "{answer}");
}

#[test]
fn supplementary_answer_covers_the_requirement_named_by_the_id() {
    for (id, criterion, requirement, artifact) in [
        (
            "SUP-REQ-X-001",
            "The first task creates `src/alpha.txt` with the nonempty line `alpha ready`.",
            "REQ-X-001",
            "src/alpha.txt",
        ),
        (
            "SUP-REQ-X-002",
            "The second task depends on the first and creates `src/beta.json` as an object whose `ready` field is true.",
            "REQ-X-002",
            "src/beta.json",
        ),
    ] {
        let prompt = format!(
            r#"{{"task":"Author exactly one acceptance entry identified below. Author ONLY entry {id}: {criterion}"}}"#
        );
        let (role, answer) = scripted_answer(&prompt);
        assert_eq!(role, "acceptance");
        let entry: serde_json::Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(entry["id"], id, "{entry}");
        assert_eq!(entry["covers"], serde_json::json!([requirement]), "{entry}");
        assert_eq!(entry["gap_permitted"], false, "{entry}");
        assert_eq!(entry["check"]["kind"], "command", "{entry}");
        assert_eq!(entry["check"]["cwd"], "repo_root", "{entry}");
        assert!(
            entry["check"]["command"]
                .as_str()
                .unwrap()
                .contains(artifact),
            "{entry}"
        );
    }
}

#[test]
fn unknown_prompt_and_unknown_acceptance_id_fail_closed() {
    let (role, _) = scripted_answer("unrecognized real prompt");
    assert_eq!(role, "unknown");
    let prompt = "Author exactly one acceptance entry identified below. Author ONLY entry AC-X-999: criterion.";
    let (role, _) = scripted_answer(prompt);
    assert_eq!(role, "unknown");

    let prompt = "Author exactly one acceptance entry identified below. Author ONLY entry AC-X-001: The outcome is ready.";
    let (role, _) = scripted_answer(prompt);
    assert_eq!(role, "unknown");
}

#[test]
fn skeleton_and_body_answers_use_the_fixture_requirement_ids() {
    let (_, skeleton) = scripted_answer("Author one complete task-skeleton JSON artifact");
    assert!(skeleton.contains("REQ-X-001"), "{skeleton}");
    assert!(skeleton.contains("REQ-X-002"), "{skeleton}");
    assert!(skeleton.contains("AC-X-001"), "{skeleton}");
    let (_, body) =
        scripted_answer("Author the complete TASK body for host-frozen task_id TASK-X-010.");
    assert!(body.contains("REQ-X-001"), "{body}");
    assert!(body.contains("AC-X-001"), "{body}");
}

#[test]
fn body_claim_sentences_name_the_artifact_and_verifier() {
    for (task_id, claims) in [
        (
            "TASK-X-010",
            &[
                ("REQ-X-001", "src/alpha.txt", "test -s src/alpha.txt"),
                ("AC-X-001", ".archon/proof/outcome.json", "python3 -c"),
            ][..],
        ),
        (
            "TASK-X-020",
            &[("REQ-X-002", "src/beta.json", "python3 -c")][..],
        ),
    ] {
        let prompt = format!("Author the complete TASK body for host-frozen task_id {task_id}.");
        let (_, body) = scripted_answer(&prompt);
        let yaml = body
            .split("```yaml\n")
            .nth(1)
            .unwrap()
            .split("\n```")
            .next()
            .unwrap();
        let parsed: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed["task_id"], task_id, "{body}");
        assert_eq!(parsed["task_id"].as_str(), Some(task_id), "{body}");
        for (claim, path, command) in claims {
            let matching_sentence = body
                .lines()
                .find(|line| line.contains(claim) && line.contains(path) && line.contains(command));
            assert!(
                matching_sentence.is_some(),
                "{claim} lacks a sentence naming {path} and {command}: {body}"
            );
        }
        assert!(yaml.contains("required_tools: ["), "{body}");
    }
}

#[test]
fn golden_answers_make_the_outcome_check_executable_and_owned() {
    let prompt = r#"{"task":"Author exactly one acceptance entry identified below. Author ONLY entry AC-X-001: The repository records a JSON outcome artifact at `.archon/proof/outcome.json` with the boolean field `ready` set to true."}"#;
    let (_, answer) = scripted_answer(prompt);
    let entry: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(entry["check"]["kind"], "command");
    assert_eq!(entry["check"]["cwd"], "repo_root");
    assert!(
        entry["check"]["command"]
            .as_str()
            .unwrap()
            .contains("[\"ready\"] is True"),
        "{entry}"
    );

    let (_, skeleton) = scripted_answer("Author one complete task-skeleton JSON artifact");
    let skeleton: serde_json::Value = serde_json::from_str(&skeleton).unwrap();
    let contracts = skeleton["tasks"][0]["deliverable_contracts"]
        .as_array()
        .unwrap();
    assert!(
        contracts
            .iter()
            .any(|contract| contract["artifact_path"] == ".archon/proof/outcome.json"),
        "{skeleton}"
    );

    let prompt = "Author the complete TASK body for host-frozen task_id TASK-X-010.";
    let (_, body) = scripted_answer(prompt);
    assert!(
        body.contains("`.archon/proof/outcome.json` — absent"),
        "{body}"
    );
    assert!(body.contains("json.load(open(sys.argv[1]))"), "{body}");
    let yaml = body
        .split("```yaml\n")
        .nth(1)
        .unwrap()
        .split("\n```")
        .next()
        .unwrap();
    let body_yaml: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).unwrap();
    assert_eq!(
        body_yaml["deliverable_contracts"]
            .as_sequence()
            .unwrap()
            .len(),
        2
    );

    let (_, judge) = scripted_answer(
        r#"{"messages":[{"role":"user","content":"Adversarially judge every acceptance check below. The toolchain is fixed: Checks: [{\"id\":\"AC-X-001\"}]"}]}"#,
    );
    let judge: serde_json::Value = serde_json::from_str(&judge).unwrap();
    assert_eq!(judge["decisions"][0]["verdict"], "accepted");
    assert!(
        !judge["decisions"][0]["counterexample"]
            .as_str()
            .unwrap()
            .trim()
            .is_empty()
    );

    let body = serde_json::json!({"messages": [{"role": "user", "content":
            "Adversarially judge every acceptance check below. The toolchain is fixed: Checks: [{\"id\":\"AC-X-001\"}]"}]});
    let (_, judge) = scripted_answer(&body.to_string());
    let judge: serde_json::Value = serde_json::from_str(&judge).unwrap();
    assert_eq!(judge["decisions"].as_array().unwrap().len(), 1);
    assert_eq!(judge["decisions"][0]["id"], "AC-X-001");
}

#[test]
fn batch_and_reask_judges_return_every_id_from_serialized_client_bodies() {
    let batch = serde_json::json!({
        "max_tokens": 1,
        "messages": [{
            "role": "user",
            "content": "Adversarially judge every acceptance check below. The toolchain is fixed: Checks: [{\"id\":\"AC-X-001\"}, {\"id\":\"SUP-REQ-X-001\"}, {\"id\":\"SUP-REQ-X-002\"}]"
        }]
    });
    let (role, answer) = scripted_answer(&batch.to_string());
    assert_eq!(role, "judge");
    let decisions: serde_json::Value = serde_json::from_str(&answer).unwrap();
    let ids = decisions["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|decision| decision["id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        ids,
        ["AC-X-001", "SUP-REQ-X-001", "SUP-REQ-X-002"]
            .into_iter()
            .collect()
    );
    for decision in decisions["decisions"].as_array().unwrap() {
        assert_eq!(decision["verdict"], "accepted");
        assert!(!decision["counterexample"].as_str().unwrap().is_empty());
        assert!(!decision["reason"].as_str().unwrap().is_empty());
    }

    let reask = serde_json::json!({
        "max_tokens": 1,
        "messages": [
            {"role": "user", "content": "Adversarially judge every acceptance check below. The toolchain is fixed: Checks: [{\"id\":\"AC-X-001\"}, {\"id\":\"SUP-REQ-X-002\"}]"},
            {"role": "assistant", "content": "{\"decisions\":[{\"id\":\"AC-X-001\"}]}"},
            {"role": "user", "content": "Adversarially judge every acceptance check below. The toolchain is fixed: Checks: [{\"id\":\"AC-X-001\"}, {\"id\":\"SUP-REQ-X-002\"}]"}
        ]
    });
    let (role, answer) = scripted_answer(&reask.to_string());
    assert_eq!(role, "judge");
    let decisions: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(decisions["decisions"].as_array().unwrap().len(), 2);
    assert_eq!(decisions["decisions"][0]["id"], "AC-X-001");
    assert_eq!(decisions["decisions"][1]["id"], "SUP-REQ-X-002");
}

#[test]
fn fidelity_critic_answers_every_obligation_in_a_serialized_provider_body() {
    use archon_workflow::fidelity_audit::{
        ClaimedObligation, ClaimingTask, parse_fidelity_response,
    };
    let fidelity_prompt = concat!(
        "You are auditing whether a task decomposition is faithful to its PRD.\n\n",
        "Obligations: [{\"id\":\"AC-X-001\",\"text\":\"ready\"},{\"id\":\"REQ-X-001\",\"text\":\"alpha\"}]\n\n",
        "FROZEN SKELETON (every task in the set, one JSON object per line):\n",
        "{\"task_id\":\"TASK-X-010\"}\n",
        "===== BEGIN TASK TASK-X-010 =====\nCreate the required outputs.\n===== END TASK TASK-X-010 ====="
    );
    let body = serde_json::json!({
        "max_tokens": 1,
        "messages": [{"role": "user", "content": fidelity_prompt}]
    })
    .to_string();
    let answer = fidelity_answer(&body).expect("serialized fidelity prompt is recognized");
    let document: serde_json::Value = serde_json::from_str(&answer).unwrap();
    let verdicts = document["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), 2, "{document}");
    assert_eq!(verdicts[0]["obligation_id"], "AC-X-001");
    assert_eq!(verdicts[1]["obligation_id"], "REQ-X-001");
    for verdict in verdicts {
        assert_eq!(verdict["necessarily_true"], true, "{verdict}");
        assert_eq!(verdict["weakest_task_id"], "", "{verdict}");
        assert_eq!(verdict["quoted_task_text"], "", "{verdict}");
        assert!(!verdict["reason"].as_str().unwrap().is_empty(), "{verdict}");
    }
    let obligations = [
        ClaimedObligation {
            id: "AC-X-001".into(),
            text: "ready".into(),
        },
        ClaimedObligation {
            id: "REQ-X-001".into(),
            text: "alpha".into(),
        },
    ];
    let tasks = [ClaimingTask {
        task_id: "TASK-X-010".into(),
        text: "Create the required outputs.".into(),
    }];
    assert!(
        parse_fidelity_response(&answer, &obligations, &tasks).is_ok(),
        "fake provider reply must satisfy the real fidelity parser: {answer}"
    );
    let false_verdict = serde_json::json!({
        "verdicts": [{
            "obligation_id": "AC-X-001",
            "necessarily_true": false,
            "weakest_task_id": "TASK-X-010",
            "reason": "The task permits the output to be absent.",
            "quoted_task_text": "Create the required outputs."
        }]
    })
    .to_string();
    assert!(parse_fidelity_response(&false_verdict, &obligations[..1], &tasks).is_ok());
    let mut bad_quote: serde_json::Value = serde_json::from_str(&false_verdict).unwrap();
    bad_quote["verdicts"][0]["quoted_task_text"] = "paraphrased loophole".into();
    assert!(parse_fidelity_response(&bad_quote.to_string(), &obligations[..1], &tasks).is_err());
    let (role, routed_answer) = scripted_answer(&body);
    assert_eq!(role, "fidelity");
    assert_eq!(routed_answer, answer);
}
