#[path = "critical_validator_repair/assembled_tests.rs"]
mod assembled_tests;
#[path = "critical_validator_repair/commit_tests.rs"]
mod commit_tests;
#[path = "critical_validator_repair/export_failure_tests.rs"]
mod export_failure_tests;
#[path = "critical_validator_repair/rejection_tests.rs"]
mod rejection_tests;
#[path = "critical_validator_repair/resume_tests.rs"]
mod resume_tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Result;
use archon_pipeline::audit::store::PipelineBundleStore;
use archon_pipeline::research::draft_artifact::write_draft_research_artifacts;
use archon_pipeline::runner::{
    AgentExecutionRequest, AgentInfo, AgentResult, LlmClient, LlmResponse, NextAgent,
    PipelineFacade, PipelineResult, PipelineSession, PipelineType, QualityScore, ToolAccessLevel,
    resume_pipeline_audited, run_pipeline, run_pipeline_audited,
};

const PAPER: &str = "# Paper\n\noriginal claim with (Smith, 2024).";
const REPAIRED: &str = "# Paper\n\nrepaired claim with (Smith, 2024).";
const REPAIRED_PADDED: &str = " \n# Paper\n\nrepaired claim with (Smith, 2024).\n ";
const FINDINGS: &str = "FAIL\nAdd the missing limitations section.";

struct RepairLlm {
    call: AtomicUsize,
    prompts: Mutex<Vec<String>>,
    requests: Mutex<
        Vec<(
            String,
            String,
            Vec<serde_json::Value>,
            Vec<serde_json::Value>,
            Vec<String>,
        )>,
    >,
    always_fail: bool,
    refuse_repair: bool,
    panic_at: Option<usize>,
    provider_fail_at: Option<usize>,
    block_persist_at: Option<(usize, PathBuf)>,
}

#[async_trait::async_trait]
impl LlmClient for RepairLlm {
    async fn run_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        self.requests.lock().unwrap().push((
            request.agent.model.clone(),
            request.agent.display_name.clone(),
            request.system.clone(),
            request.tools.clone(),
            request.allowed_tools.clone(),
        ));
        let model = request.agent.model.clone();
        self.send_message(request.messages, request.system, request.tools, &model)
            .await
    }

    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        let prompt = messages[0]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        self.prompts.lock().unwrap().push(prompt.clone());
        let call = self.call.fetch_add(1, Ordering::SeqCst);
        if self.panic_at == Some(call) {
            panic!("simulated process interruption during repair");
        }
        if self.provider_fail_at == Some(call) {
            anyhow::bail!("simulated provider failure");
        }
        if let Some((blocked_call, path)) = &self.block_persist_at
            && *blocked_call == call
        {
            std::fs::create_dir_all(path)?;
        }
        let content = match call {
            0 => PAPER,
            1 | 5 => FINDINGS,
            2 if self.refuse_repair => "I cannot help with that request.",
            2 | 4 => REPAIRED_PADDED,
            3 if !self.always_fail => "PASS",
            3 => FINDINGS,
            _ => FINDINGS,
        };
        Ok(LlmResponse {
            content: content.to_string(),
            tool_uses: Vec::new(),
            tokens_in: Some(17),
            tokens_out: None,
            stop_reason: None,
        })
    }
}

struct ValidatorFacade {
    session_id: String,
    fail_prompt_attempt: Option<u8>,
    panic_repair_prompt: bool,
    fail_repair_prompt: bool,
}

#[async_trait::async_trait]
impl PipelineFacade for ValidatorFacade {
    async fn init_session(&self, task: &str) -> Result<PipelineSession> {
        Ok(PipelineSession {
            id: self.session_id.clone(),
            pipeline_type: if self.session_id.starts_with("coding-") {
                PipelineType::Coding
            } else {
                PipelineType::Research
            },
            task: task.to_string(),
            started_at: Instant::now(),
            agent_results: Vec::new(),
            leann_context: String::new(),
        })
    }

    async fn next_agent(&self, session: &PipelineSession) -> Result<NextAgent> {
        if self.session_id.starts_with("assembled-") {
            let agent = match session.agent_results.len() {
                0 => Some(agent("chapter-output")),
                1 => Some(agent("paper")),
                2 => Some(agent("validator")),
                _ => None,
            };
            return Ok(agent.map(NextAgent::Continue).unwrap_or(NextAgent::Done));
        }
        let agent = match session.agent_results.len() {
            0 => Some(agent("paper")),
            1 => Some(agent("validator")),
            _ => None,
        };
        Ok(agent.map(NextAgent::Continue).unwrap_or(NextAgent::Done))
    }

    async fn build_prompt(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
    ) -> Result<(
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
    )> {
        let content = if agent.key == "validator" {
            session.agent_results[0].1.output.clone()
        } else {
            "write paper".to_string()
        };
        Ok((
            vec![serde_json::json!({"role":"user", "content":content})],
            vec![
                serde_json::json!({"type":"text", "text":format!("Normal system profile for {}", agent.display_name)}),
            ],
            vec![],
        ))
    }

    async fn build_prompt_for_attempt(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
        attempt: u8,
    ) -> Result<(
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
    )> {
        if agent.key == "validator" && self.fail_prompt_attempt == Some(attempt) {
            anyhow::bail!("simulated prompt construction interruption");
        }
        self.build_prompt(session, agent).await
    }

    async fn score_quality(
        &self,
        _session: &PipelineSession,
        agent: &AgentInfo,
        result: &AgentResult,
    ) -> Result<QualityScore> {
        let overall = if agent.key == "validator" && result.output.starts_with("FAIL") {
            0.0
        } else if agent.key == "paper" && result.output.starts_with("I cannot") {
            0.0
        } else {
            0.9
        };
        Ok(QualityScore {
            overall,
            dimensions: HashMap::new(),
        })
    }

    fn critical_validator_artifact(&self, agent: &AgentInfo) -> Option<String> {
        (agent.key == "validator" && !self.session_id.starts_with("coding-"))
            .then(|| "paper".to_string())
    }

    fn critical_validator_artifact_text(
        &self,
        session: &PipelineSession,
        validator: &AgentInfo,
    ) -> Option<String> {
        let artifact_key = self.critical_validator_artifact(validator)?;
        let judged_key = if self.session_id.starts_with("assembled-") {
            "chapter-output"
        } else {
            artifact_key.as_str()
        };
        session
            .agent_results
            .iter()
            .find(|(agent, _)| agent.key == judged_key)
            .map(|(_, result)| result.output.clone())
    }

    fn build_critical_repair_prompt(
        &self,
        _session: &PipelineSession,
        _validator: &AgentInfo,
        artifact: &str,
        findings: &str,
    ) -> Result<String> {
        if self.panic_repair_prompt {
            panic!("simulated process interruption during repair prompt construction");
        }
        if self.fail_repair_prompt {
            anyhow::bail!("simulated repair prompt construction failure");
        }
        Ok(format!(
            "Repair only from exact findings. Preserve citations and supported claims.\n\nCurrent paper:\n{artifact}\n\nExact findings:\n{findings}"
        ))
    }

    fn write_failed_critical_draft(
        &self,
        _session: &PipelineSession,
        artifact: &str,
        findings: &str,
        bundle_dir: &Path,
    ) -> Result<Option<Vec<(String, PathBuf, String)>>> {
        Ok(Some(write_draft_research_artifacts(
            bundle_dir, artifact, findings,
        )?))
    }

    async fn process_completion(
        &self,
        _session: &mut PipelineSession,
        _agent: &AgentInfo,
        _result: &AgentResult,
        _quality: &QualityScore,
    ) -> Result<()> {
        Ok(())
    }

    async fn finalize(&self, session: PipelineSession) -> Result<PipelineResult> {
        if let Some(path) = session.task.strip_prefix("block-final-export=") {
            std::fs::create_dir_all(path)?;
        }
        if let Some(path) = session.task.strip_prefix("block-audit-complete=") {
            let audit_log = Path::new(path);
            std::fs::remove_file(audit_log)?;
            std::fs::create_dir(audit_log)?;
        }
        Ok(PipelineResult {
            session_id: session.id,
            pipeline_type: session.pipeline_type,
            agent_results: session.agent_results,
            total_cost_usd: 0.0,
            duration: session.started_at.elapsed(),
            final_output: "# Repaired Paper\n\n## Abstract\n\nValidated final paper content.\n\n## Introduction\n\nThis paper describes a validated result.\n\n## References\n\nSmith, J. (2024). Example."
                .to_string(),
        })
    }

    async fn after_run_completed(
        &self,
        bundle_dir: &Path,
    ) -> Result<Vec<(String, PathBuf, String)>> {
        if self.session_id.starts_with("coding-") {
            return Ok(Vec::new());
        }
        archon_pipeline::research::draft_artifact::supersede_draft_research_artifacts(bundle_dir)
    }
}

fn agent(key: &str) -> AgentInfo {
    AgentInfo {
        key: key.to_string(),
        display_name: key.to_string(),
        model: if key == "paper" {
            "artifact-model"
        } else {
            "validator-model"
        }
        .to_string(),
        phase: 1,
        critical: key == "validator",
        parallelizable: false,
        quality_threshold: 0.5,
        tool_access_level: ToolAccessLevel::ReadOnly,
    }
}

fn llm(always_fail: bool) -> Arc<RepairLlm> {
    Arc::new(RepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
        requests: Mutex::new(Vec::new()),
        always_fail,
        refuse_repair: false,
        panic_at: None,
        provider_fail_at: None,
        block_persist_at: None,
    })
}

fn facade_ref(facade: &ValidatorFacade) -> &dyn PipelineFacade {
    facade
}

struct ResumeRepairLlm {
    call: AtomicUsize,
    prompts: Mutex<Vec<String>>,
}

struct ResumeBudgetLlm {
    call: AtomicUsize,
}

#[async_trait::async_trait]
impl LlmClient for ResumeBudgetLlm {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        let prompt = messages[0]["content"].as_str().unwrap_or_default();
        let call = self.call.fetch_add(1, Ordering::SeqCst);
        let content = if prompt.contains("Repair only from exact findings") {
            REPAIRED
        } else if call == 0 {
            "FAIL\nSecond attempt still needs changes."
        } else {
            "PASS"
        };
        Ok(LlmResponse {
            content: content.to_string(),
            tool_uses: Vec::new(),
            tokens_in: None,
            tokens_out: None,
            stop_reason: None,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for ResumeRepairLlm {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        self.prompts.lock().unwrap().push(
            messages[0]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
        let call = self.call.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            content: if call == 0 { REPAIRED } else { "PASS" }.to_string(),
            tool_uses: Vec::new(),
            tokens_in: None,
            tokens_out: None,
            stop_reason: None,
        })
    }
}

#[tokio::test]
async fn failed_validation_repairs_artifact_before_next_prompt() {
    let facade = ValidatorFacade {
        session_id: "repair-success".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let llm = llm(false);
    run_pipeline(facade_ref(&facade), llm.as_ref(), "task", None, None, None)
        .await
        .expect("repaired paper should pass");
    let prompts = llm.prompts.lock().unwrap();
    assert!(prompts[2].contains("Current paper:\n"));
    assert!(prompts[2].contains(PAPER));
    assert!(prompts[2].contains(FINDINGS));
    assert!(
        prompts[3].contains(REPAIRED),
        "next validator attempt must judge repaired paper"
    );
    let requests = llm.requests.lock().unwrap();
    assert_eq!(requests[2].0, "artifact-model");
    assert_eq!(requests[2].1, "paper");
    assert_eq!(requests[2].2[0]["text"], "Normal system profile for paper");
    assert_eq!(requests[2].3, requests[0].3);
    assert_eq!(requests[2].4, requests[0].4);
}

#[tokio::test]
async fn final_failed_validation_writes_draft_and_findings_only() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "repair-failure".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let llm = llm(true);
    assert!(
        run_pipeline_audited(&facade, llm.as_ref(), "task", tmp.path(), None, None, None)
            .await
            .is_err()
    );
    let bundle = PipelineBundleStore::new(tmp.path()).bundle_dir("repair-failure");
    let draft = std::fs::read_to_string(bundle.join("exports/draft-paper.md")).unwrap();
    let findings = std::fs::read_to_string(bundle.join("exports/draft-paper.findings.md")).unwrap();
    assert!(draft.contains("FAILED VALIDATION"));
    assert!(draft.contains(REPAIRED));
    assert!(findings.contains(FINDINGS));
    assert!(!bundle.join("exports/final-paper.md").exists());
    let events = std::fs::read_to_string(bundle.join("audit.log")).unwrap();
    assert_eq!(
        events
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter(|event| event["type"] == "artifact_repair_committed")
            .count(),
        2
    );
    assert!(events.contains("research-paper-draft-findings"));
    let attempt_events = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|event| event["type"] == "llm_attempt_completed")
        .collect::<Vec<_>>();
    assert!(
        attempt_events
            .iter()
            .any(|event| event["tokens_in"] == 17 && event["tokens_out"].is_null())
    );
    let repair = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["type"] == "artifact_repair_committed")
        .unwrap();
    assert!(
        repair["output_path"]
            .as_str()
            .unwrap()
            .starts_with("outputs/repairs/")
    );
    let stored = std::fs::read(bundle.join(repair["output_path"].as_str().unwrap())).unwrap();
    assert_eq!(
        repair["output_hash"].as_str().unwrap(),
        archon_pipeline::audit::store::sha256_hex(stored)
    );
    let verification = archon_pipeline::audit::verify_bundle(
        &PipelineBundleStore::new(tmp.path()),
        "repair-failure",
        false,
    )
    .unwrap();
    assert!(verification.valid, "{:#?}", verification.findings);
}
