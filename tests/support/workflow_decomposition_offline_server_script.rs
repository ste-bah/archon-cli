pub(super) fn scripted_answer(prompt: &str) -> (&'static str, String) {
    if let Some(answer) = fidelity_answer(prompt) {
        return ("fidelity", answer);
    }
    if prompt.contains("Adversarially judge every acceptance check") || prompt.contains("decisions")
    {
        // `prompt` is the serialized request body, so a quoted id inside the
        // message text reads `\"AC-X-001\"`: drop the escape before the quote.
        let ids = prompt
            .split('"')
            .map(|part| part.trim_end_matches('\\'))
            .filter(|part| part.starts_with("AC-") || part.starts_with("SUP-"))
            .collect::<std::collections::BTreeSet<_>>();
        let decisions = ids
            .into_iter()
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "verdict": "accepted",
                    "counterexample": "None found: a missing or false outcome makes the command exit non-zero.",
                    "reason": "The command directly checks the declared outcome and exits non-zero when it is false."
                })
            })
            .collect::<Vec<_>>();
        return (
            "judge",
            serde_json::json!({"decisions": decisions}).to_string(),
        );
    }
    if prompt.contains("Author exactly one acceptance entry") {
        let id = prompt
            .split("Author ONLY entry ")
            .nth(1)
            .and_then(|s| s.split(':').next())
            .unwrap_or("AC-001");
        if id.starts_with("SUP-REQ-X-001") {
            return (
                "acceptance",
                serde_json::json!({
                    "id": id,
                    "criterion": "The first artifact contains the required line.",
                    "check": {"kind": "command", "command": "test -f src/alpha.txt && grep -qx 'alpha ready' src/alpha.txt", "cwd": "repo_root"},
                    "gap_permitted": false,
                    "covers": ["REQ-X-001"],
                    "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
                }).to_string(),
            );
        }
        if id.starts_with("SUP-REQ-X-002") {
            return (
                "acceptance",
                serde_json::json!({
                    "id": id,
                    "criterion": "The second artifact is ready JSON.",
                    "check": {"kind": "command", "command": "python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"ready\"] is True' src/beta.json", "cwd": "repo_root"},
                    "gap_permitted": false,
                    "covers": ["REQ-X-002"],
                    "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
                }).to_string(),
            );
        }
        return (
            "acceptance",
            serde_json::json!({
                "id": id,
                "criterion": "The repository records a JSON outcome artifact at `.archon/proof/outcome.json` with the boolean field `ready` set to true.",
                "check": {"kind": "command", "command": "python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"ready\"] is True' .archon/proof/outcome.json", "cwd": "repo_root"},
                "gap_permitted": false,
                "covers": [],
                "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
            }).to_string(),
        );
    }
    if prompt.contains("Author one complete task-skeleton JSON artifact") {
        return (
            "skeleton",
            serde_json::json!({
                "schema_version": 1,
                "acceptance_digest": "",
                "tasks": [
                    {
                        "task_id": "TASK-X-010", "file_name": "TASK-X-010.md", "depends_on": [], "blocks": ["TASK-X-020"], "implements": ["REQ-X-001", "AC-X-001"],
                        "deliverable_contracts": [
                            {"kind": "alpha_text", "artifact_path": "src/alpha.txt", "artifact_format": "text", "min_instances": 1},
                            {"kind": "generic_outcome", "artifact_path": ".archon/proof/outcome.json", "artifact_format": "json", "required_true_fields": ["ready"], "min_instances": 1}
                        ]
                    },
                    {
                        "task_id": "TASK-X-020", "file_name": "TASK-X-020.md", "depends_on": [{"task_id": "TASK-X-010", "consumes": [{"artifact_path": "src/alpha.txt"}], "ordering_only": false}], "blocks": [], "implements": ["REQ-X-002"],
                        "deliverable_contracts": [{"kind": "beta_json", "artifact_path": "src/beta.json", "artifact_format": "json", "required_true_fields": ["ready"], "min_instances": 1}]
                    }
                ]
            }).to_string(),
        );
    }
    if prompt.contains("Author the complete TASK body") {
        let second = prompt.contains("task_id TASK-X-020");
        let (id, title, dep, blocks, req, path, text, contracts) = if second {
            (
                "TASK-X-020",
                "Create beta data",
                "\n  - task_id: TASK-X-010\n    consumes:\n      - artifact_path: src/alpha.txt\n    ordering_only: false",
                "[]",
                "REQ-X-002",
                "src/beta.json",
                "Write a JSON object with ready set to true. REQ-X-002 is verified by `python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"ready\"] is True' src/beta.json`.",
                "- kind: beta_json\n    artifact_path: src/beta.json\n    artifact_format: json\n    required_true_fields: [ready]\n    min_instances: 1",
            )
        } else {
            (
                "TASK-X-010",
                "Create alpha data",
                "[]",
                "[TASK-X-020]",
                "REQ-X-001, AC-X-001",
                "src/alpha.txt",
                "Write the nonempty line alpha ready and create the JSON outcome artifact with ready set to true. REQ-X-001 is verified by `test -s src/alpha.txt`. AC-X-001 is verified by `python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"ready\"] is True' .archon/proof/outcome.json`.",
                "- kind: alpha_text\n    artifact_path: src/alpha.txt\n    artifact_format: text\n    min_instances: 1\n  - kind: generic_outcome\n    artifact_path: .archon/proof/outcome.json\n    artifact_format: json\n    required_true_fields: [ready]\n    min_instances: 1",
            )
        };
        let observations = if second {
            format!("`{path}` — absent")
        } else {
            format!("`{path}` — absent\n`.archon/proof/outcome.json` — absent")
        };
        let focused_tests = if second {
            format!(
                "- `python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"ready\"] is True' {path}`"
            )
        } else {
            format!(
                "- `test -s {path}`\n- `python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"ready\"] is True' .archon/proof/outcome.json`"
            )
        };
        return (
            "body",
            format!(
                "```yaml\ntask_id: {id}\ntitle: {title}\ncomplexity: low\nstatus: ready\ndepends_on: {dep}\nblocks: {blocks}\nimplements: [{req}]\nrequired_env_keys: []\nrequired_tools: {}\ndeliverable_contracts:\n  {contracts}\n```\n\n## Objective\n{text}\n\n## Files Expected to Change\n{observations}\n\n## Focused Tests\n{focused_tests}\n",
                if second {
                    "[python3]"
                } else {
                    "[test, python3]"
                }
            ),
        );
    }
    if prompt.contains("audit") || prompt.contains("Audit") {
        return ("audit", r#"{"verdict":"pass","findings":[]}"#.into());
    }
    (
        "unexpected",
        r#"{"error":"unscripted fake provider request"}"#.into(),
    )
}

fn fidelity_answer(serialized_request: &str) -> Option<String> {
    let request: serde_json::Value = serde_json::from_str(serialized_request).ok()?;
    let messages = request.get("messages")?.as_array()?;
    let content = messages
        .iter()
        .filter_map(|message| message.get("content").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if !content.contains("You are auditing whether a task decomposition is faithful to its PRD") {
        return None;
    }
    let obligations_text = content.split_once("Obligations: ")?.1;
    let obligations_json = obligations_text.split_once("\n\n")?.0;
    let obligations: Vec<serde_json::Value> = serde_json::from_str(obligations_json).ok()?;
    let verdicts = obligations
        .iter()
        .map(|obligation| {
            serde_json::json!({
                "obligation_id": obligation["id"],
                "necessarily_true": true,
                "weakest_task_id": "",
                "reason": "The task requires the named deliverable in the project and its focused check verifies it.",
                "quoted_task_text": ""
            })
        })
        .collect::<Vec<_>>();
    Some(serde_json::json!({"verdicts": verdicts}).to_string())
}

#[cfg(test)]
mod tests {
    use super::{fidelity_answer, scripted_answer};

    #[test]
    fn routes_current_acceptance_author_prompt_and_extracts_its_id() {
        let prompt = r#"{"task":"Author exactly one acceptance entry identified below. Author ONLY entry AC-X-001: The outcome is ready."}"#;
        let (role, answer) = scripted_answer(prompt);
        assert_eq!(role, "acceptance");
        assert!(answer.contains(r#""id":"AC-X-001""#), "{answer}");
    }

    #[test]
    fn supplementary_answer_covers_the_requirement_named_by_the_id() {
        for (id, requirement, artifact) in [
            ("SUP-REQ-X-001", "REQ-X-001", "src/alpha.txt"),
            ("SUP-REQ-X-002", "REQ-X-002", "src/beta.json"),
        ] {
            let prompt = format!(
                r#"{{"task":"Author exactly one acceptance entry identified below. Author ONLY entry {id}: Check {artifact}."}}"#
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
    fn skeleton_and_body_answers_use_the_fixture_requirement_ids() {
        let (_, skeleton) = scripted_answer("Author one complete task-skeleton JSON artifact");
        assert!(skeleton.contains("REQ-X-001"), "{skeleton}");
        assert!(skeleton.contains("REQ-X-002"), "{skeleton}");
        assert!(skeleton.contains("AC-X-001"), "{skeleton}");
        let (_, body) = scripted_answer("Author the complete TASK body");
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
            let prompt =
                format!("Author the complete TASK body for host-frozen task_id {task_id}.");
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
                let matching_sentence = body.lines().find(|line| {
                    line.contains(claim) && line.contains(path) && line.contains(command)
                });
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
            r#"{"task":"Adversarially judge every acceptance check. decisions include ","id":"AC-X-001"}"#,
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
            "Adversarially judge every acceptance check below. {\"id\":\"AC-X-001\"}"}]});
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
                "content": "Adversarially judge every acceptance check. Checks: [{\"id\":\"AC-X-001\"}, {\"id\":\"SUP-REQ-X-001\"}, {\"id\":\"SUP-REQ-X-002\"}]"
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
                {"role": "user", "content": "Judge the full batch: [{\"id\":\"AC-X-001\"}, {\"id\":\"SUP-REQ-X-002\"}]"},
                {"role": "assistant", "content": "{\"decisions\":[{\"id\":\"AC-X-001\"}]}"},
                {"role": "user", "content": "Retry the full batch. decisions must include SUP-REQ-X-002 and AC-X-001."}
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
        assert!(
            parse_fidelity_response(&bad_quote.to_string(), &obligations[..1], &tasks).is_err()
        );
        let (role, routed_answer) = scripted_answer(&body);
        assert_eq!(role, "fidelity");
        assert_eq!(routed_answer, answer);
    }
}
