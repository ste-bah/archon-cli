pub(super) fn scripted_answer(prompt: &str) -> (&'static str, String) {
    let text = user_prompt_text(prompt).unwrap_or_default();
    // Exact markers: acceptance src/command/workflow_decompose_v1_acceptance.js:24; skeleton/body src/command/workflow_decompose_v1.js:164/231;
    // judge src/command/workflow_task_set_judge.rs:50; passability judge
    // src/command/workflow_acceptance_passability.rs:55; fidelity crates/archon-workflow/src/fidelity_audit.rs:200.
    if text.contains("You are auditing whether a task decomposition is faithful to its PRD.")
        && let Some(answer) = fidelity_answer(prompt)
    {
        return ("fidelity", answer);
    }
    if text.contains("Adversarially judge every acceptance check below. The toolchain is fixed:")
        || text.starts_with("Each acceptance check below already ran once on the repository tree BEFORE any implementation of its criterion")
    {
        let Some(checks) = text
            .rsplit_once("Checks: ")
            .and_then(|(_, json)| serde_json::from_str::<Vec<serde_json::Value>>(json).ok())
        else {
            return unknown_prompt();
        };
        let ids = checks
            .iter()
            .filter_map(|check| check.get("id").and_then(serde_json::Value::as_str))
            .collect::<std::collections::BTreeSet<_>>();
        if ids.is_empty()
            || ids
                .iter()
                .any(|id| !matches!(*id, "AC-X-001" | "SUP-REQ-X-001" | "SUP-REQ-X-002"))
        {
            return unknown_prompt();
        }
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
    if text.contains("Author exactly one acceptance entry identified below") {
        let Some(id) = text
            .split_once("Author ONLY entry ")
            .and_then(|(_, s)| s.split_once(':').map(|(id, _)| id.trim()))
        else {
            return unknown_prompt();
        };
        let criterion = match id {
            "AC-X-001" => {
                "The repository records a JSON outcome artifact at `.archon/proof/outcome.json` with the boolean field `ready` set to true."
            }
            // A supplementary entry's authored text names the PRD requirement
            // it covers (src/command/workflow_decompose_v1_acceptance.js:198;
            // tests/fixtures/offline_decomposition/prd.md:9-10).
            "SUP-REQ-X-001" => {
                "SUPPLEMENTARY check owed to PRD requirement REQ-X-001 (no other check covers it): The first task creates `src/alpha.txt` with the nonempty line `alpha ready`."
            }
            "SUP-REQ-X-002" => {
                "SUPPLEMENTARY check owed to PRD requirement REQ-X-002 (no other check covers it): The second task depends on the first and creates `src/beta.json` as an object whose `ready` field is true."
            }
            _ => return unknown_prompt(),
        };
        // The criterion must be the text authored for this id, not a mention
        // of it elsewhere in the prompt.
        let authored = text
            .split_once(&format!("Author ONLY entry {id}:"))
            .map(|(_, rest)| rest.trim_start())
            .unwrap_or_default();
        if !authored.starts_with(criterion) {
            return unknown_prompt();
        }
        if id == "SUP-REQ-X-001" {
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
        if id == "SUP-REQ-X-002" {
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
    if text.contains("Author one complete task-skeleton JSON artifact") {
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
    if text.contains("Author the complete TASK body for host-frozen task_id ") {
        let Some(id) = text
            .split_once("Author the complete TASK body for host-frozen task_id ")
            .and_then(|(_, rest)| rest.split_once('.').map(|(id, _)| id.trim()))
        else {
            return unknown_prompt();
        };
        if !matches!(id, "TASK-X-010" | "TASK-X-020") {
            return unknown_prompt();
        }
        let second = id == "TASK-X-020";
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
    unknown_prompt()
}

fn unknown_prompt() -> (&'static str, String) {
    (
        "unknown",
        r#"{"error":"unscripted fake provider request"}"#.into(),
    )
}

fn user_prompt_text(serialized_request: &str) -> Option<String> {
    let Ok(request) = serde_json::from_str::<serde_json::Value>(serialized_request) else {
        return Some(serialized_request.to_owned());
    };
    let Some(messages) = request
        .get("messages")
        .and_then(serde_json::Value::as_array)
    else {
        return Some(serialized_request.to_owned());
    };
    Some(
        messages
            .iter()
            .filter_map(|message| message.get("content").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
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
#[path = "workflow_decomposition_offline_server_script_tests.rs"]
mod tests;
