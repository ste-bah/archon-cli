use super::*;
use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use std::os::unix::fs::PermissionsExt;

const OLD_INSTRUCTION: &str = "A check that names its deciding file by a literal path relative to its working directory and whose named_files assert the criterion is a valid regression guard. A counterexample against such a guard must show that the named file's assertions pass while the criterion is false.";
const MASKED_CHECK: &str =
    "./verify checks/input; test -f checks/input && grep -q ready checks/input";

/// Execute the proposed counterexample, then apply the instruction's rule to
/// the observed status dependency. Replies go through the production judge.
struct DependencyJudge(std::path::PathBuf);

#[async_trait::async_trait]
impl WorkflowLlmClient for DependencyJudge {
    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        assert_eq!(temperature, 0.0);
        let prompt = messages[0]["content"].as_str().unwrap();
        let checks = prompt_json(prompt);
        let check = &checks[0];
        let command = check["check"]["command"].as_str().unwrap();
        let files = check["named_files"].as_array().unwrap();
        assert!(files.iter().any(|file| {
            file["path"] == "checks/input"
                && file["status"] == "supplied"
                && file["content"].as_str().unwrap().contains("ready")
        }));
        let deciding = command.split_once(';').map_or(command, |(first, _)| first);
        let status = |text: &str| {
            std::process::Command::new("sh")
                .args(["-c", text])
                .current_dir(&self.0)
                .status()
                .unwrap()
                .success()
        };
        let masked_failure = !status(deciding) && status(command);
        let requires_dependency = prompt.contains("it fails whenever that test fails");
        let verdict = if masked_failure && requires_dependency {
            "refuted"
        } else {
            "accepted"
        };
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"decisions": [{
                "id": "AC-X-001", "verdict": verdict,
                "counterexample": "the existing assertion fails while the final file checks succeed",
                "reason": "the complete command's status is measured against the deciding assertion"
            }]}).to_string(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the judge samples explicitly")
    }
}

fn fixture() -> (tempfile::TempDir, JudgeRoots) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("checks")).unwrap();
    std::fs::write(
        root.path().join("checks/input"),
        "ready\nassert actual equals expected\n",
    )
    .unwrap();
    let executable = root.path().join("verify");
    std::fs::write(&executable, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let roots = JudgeRoots {
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    (root, roots)
}

#[tokio::test]
async fn named_file_judge_refutes_a_sequence_that_discards_the_deciding_status() {
    let (root, roots) = fixture();
    let client = DependencyJudge(root.path().to_path_buf());
    let subject = command_check(MASKED_CHECK, TrustedCwd::ProjectRoot);
    let judged = judge_entries_with_roots(&client, subject.clone(), "", &roots)
        .await
        .unwrap();
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );

    let legacy = judge_prompted(
        &client,
        subject,
        |subset| {
            batched_judge_prompt_with_roots(subset, &roots)
                .map(|prompt| prompt.replace(NAMED_FILES_INSTRUCTION, OLD_INSTRUCTION))
        },
        "",
        require_judged_prose,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        legacy.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );

    let dependent = command_check(
        "./verify checks/input && test -f checks/input && grep -q ready checks/input",
        TrustedCwd::ProjectRoot,
    );
    let judged = judge_entries_with_roots(&client, dependent, "", &roots)
        .await
        .unwrap();
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
}
