use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use archon_pipeline::audit::store::PipelineBundleStore;
use archon_pipeline::runner::{LlmClient, LlmResponse, PipelineFacade, run_pipeline_audited};

use super::{FINDINGS, REPAIRED, ValidatorFacade};

struct AssembledRepairLlm {
    call: AtomicUsize,
    prompts: Mutex<Vec<String>>,
    exhaust: bool,
}

#[async_trait::async_trait]
impl LlmClient for AssembledRepairLlm {
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
        self.prompts.lock().unwrap().push(prompt);
        let call = self.call.fetch_add(1, Ordering::SeqCst);
        let content = if self.exhaust {
            match call {
                0 => "Assembled chapter paper judged by validator.",
                1 => "Different stored combiner output.",
                call if call >= 2 && call % 2 == 1 => REPAIRED,
                _ => FINDINGS,
            }
        } else {
            match call {
                0 => "Assembled chapter paper judged by validator.",
                1 => "Different stored combiner output.",
                2 => FINDINGS,
                3 => REPAIRED,
                _ => "PASS",
            }
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

#[tokio::test]
async fn repair_uses_the_assembled_text_judged_and_hashes_that_input() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "assembled-repair".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let llm = AssembledRepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
        exhaust: false,
    };
    run_pipeline_audited(
        &facade as &dyn PipelineFacade,
        &llm,
        "task",
        tmp.path(),
        None,
        None,
        None,
    )
    .await
    .expect("repair of assembled paper should pass");

    let prompts = llm.prompts.lock().unwrap();
    assert!(prompts[3].contains("Assembled chapter paper judged by validator."));
    assert!(!prompts[3].contains("Different stored combiner output."));
    let bundle = PipelineBundleStore::new(tmp.path()).bundle_dir("assembled-repair");
    let events = std::fs::read_to_string(bundle.join("audit.log")).unwrap();
    let repair = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["type"] == "artifact_repair_committed")
        .unwrap();
    assert_eq!(
        repair["input_hash"].as_str().unwrap(),
        archon_pipeline::audit::store::sha256_hex(
            "Assembled chapter paper judged by validator.".as_bytes()
        )
    );
}

#[tokio::test]
async fn exhausted_validator_draft_uses_assembled_text_judged() {
    let tmp = tempfile::tempdir().unwrap();
    let facade = ValidatorFacade {
        session_id: "assembled-draft-exhaustion".into(),
        fail_prompt_attempt: None,
        panic_repair_prompt: false,
        fail_repair_prompt: false,
    };
    let llm = AssembledRepairLlm {
        call: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
        exhaust: true,
    };
    assert!(
        run_pipeline_audited(
            &facade as &dyn PipelineFacade,
            &llm,
            "task",
            tmp.path(),
            None,
            None,
            None,
        )
        .await
        .is_err()
    );
    let bundle = PipelineBundleStore::new(tmp.path()).bundle_dir("assembled-draft-exhaustion");
    let draft = std::fs::read_to_string(bundle.join("exports/draft-paper.md")).unwrap();
    assert!(draft.contains("Assembled chapter paper judged by validator."));
    assert!(!draft.contains("Different stored combiner output."));
}
