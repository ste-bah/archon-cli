use super::*;
use archon_llm::provider::LlmRequest;
use sha2::{Digest, Sha256};

struct EnvelopeCritic {
    inner: Arc<Critic>,
    system_text: &'static str,
}

impl EnvelopeCritic {
    fn system(&self, mut system: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
        system.push(serde_json::json!({"type":"text", "text":self.system_text}));
        system
    }
}

#[async_trait]
impl WorkflowLlmClient for EnvelopeCritic {
    fn message_request_identity(&self, request: &LlmRequest) -> Option<String> {
        let body = serde_json::json!({
            "messages":request.messages,
            "system":self.system(request.system.clone()),
            "model":request.model,
            "temperature":request.extra["temperature"],
        });
        Some(hex::encode(Sha256::digest(body.to_string())))
    }

    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the audit pins temperature")
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.inner
            .send_message_with_temperature(messages, self.system(system), tools, model, temperature)
            .await
    }
}

#[tokio::test]
async fn retry_reasks_after_effective_system_changes_at_the_same_config_and_head() {
    let temp = corpus();
    let critic = Critic::new();
    for (system_text, expected_calls) in [("clean", 3), ("clean", 3), ("spoof", 6)] {
        let envelope = Arc::new(EnvelopeCritic {
            inner: critic.clone(),
            system_text,
        });
        // Same legacy identity and binary revision throughout; the actual
        // request envelope alone decides whether these verdicts can be reused.
        assert!(envelope.request_identity().is_none());
        let evaluation = evaluate_lint_with_fidelity_resumable(
            temp.path(),
            &LintSource::Tasks(temp.path().join("tasks/PRD-QX-001")),
            archon_core::config::GateMode::Enforce,
            Ok(envelope),
            &[],
            &FreezeResume::none(),
        )
        .await
        .unwrap();
        assert!(
            evaluation.operational_error().is_none(),
            "{}",
            evaluation.report
        );
        assert_eq!(critic.calls(), expected_calls);
    }
}

#[tokio::test]
async fn an_unknown_envelope_is_reasked_without_claiming_saved_progress() {
    let temp = corpus();
    let mut critic = Critic::new();
    Arc::get_mut(&mut critic).unwrap().request = None;
    let resume = FreezeResume::none();
    complete(temp.path(), critic.clone(), &resume).await;
    complete(temp.path(), critic.clone(), &resume).await;
    assert_eq!(critic.calls(), 6);
    assert!(records(temp.path()).is_empty());
    assert_eq!(resume.progress.total(), 0);
}

/// The catalog identity changes with #356 (schema 2: timeout semantics); rebuilding
/// schema 1 reproduces the old catalog. The fixed script digest moves by design
/// (Issues 261, 288 four times, 337, 357, 360, 366, 379, 381, 383, 384, 385, 389, 392, 423, 441 remediation body forcing); older runs resume via #358 and the #360 seed.
#[test]
fn issue356_catalog_identity_changes_and_script_matches_the_release() {
    let catalog =
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev").unwrap();
    let old_digest = "83b6c39e67e41a9f0fcd6fececa0e009379ac534bf5e511e8d2ded33afa24472";
    let mut old = catalog.clone();
    old.schema_version = 1;
    let landing = old.capabilities.get_mut("land-task-body").unwrap();
    let context_arg = landing
        .argv_template
        .windows(2)
        .position(|pair| pair[0] == "--task-context-file")
        .unwrap();
    landing.argv_template.drain(context_arg..context_arg + 2);
    old.recompute_digest().unwrap();
    assert_eq!(old.digest, old_digest);
    assert_ne!(
        catalog.digest, old_digest,
        "timeout semantics must change resume identity"
    );
    // #472 re-pin only: reply unwrapping and mixed-form refusal text; verdict rules unchanged.
    // #476 re-pin only: block-form refusal guidance changed; verdict rules unchanged.
    // #497: stalled bodies stay pending until their siblings settle, then pause once;
    // an unusable freeze judge reply retries inside the ordinary no-progress window.
    // F8b: a pause keeps its caller's recovery and a pending body its earlier note;
    // C4: a stale acceptance freeze rebuilds its candidate from the current contract.
    // R3b: a stale rebuild keeps the loop's unpublished drafts (seed path too); guard-proof text.
    // Review merge: the decomposition author gets the host re-author's check-path rule byte for byte, plus the absent/not-supplied input rule.
    // D5: a set gate's unusable judge reply counts in the ordinary no-progress window, not the host's operational one.
    // O1/O3: a skeleton re-author re-authors only the bodies its change reaches; the dropped-task check is an assertion.
    // O3 at launch: a skeleton verified in place keys kept bodies on the launcher's frozen entries and on-disk bodies.
    // H1: an operational (host-unproven) finding beside an authored finding stops nothing; alone it still stops the phase.
    // Issue 495: check-path guidance reflects the copied repository sibling alias.
    // Issues 529/530: a shrinking failing-subject set is progress; a max_tokens retry names its size.
    // Issue 494: acceptance author prompts carry the named-files block before the reply format.
    // Windows CPU fix re-pin only: native byte counting and incremental JSON sizing preserve bytes.
    // Issue 551: repair prompts list the latest open findings as current; earlier ones only as do-not-regress history.
    // Issue 552: entry prompts list only their findings open in the latest gate result; a judge failure reaches only the entries it names, each with its own known finding.
    // Issue 554: an operational finding on an authored entry never stops the phase; it is re-frozen, then routed to its author; skeletonPolicy and unprovenAuthored moved into the frozen module.
    // Issue 555: an entry prompt carries only its own findings and phase-level ones (labelled, earlier ones as do-not-regress); the shared part names none; entry rung 3 names its own findings; earlier-findings sections are bounded with the full list in a context file.
    // Issue 559: the acceptance reply format shows covers; a re-authored entry prompt states its current covers as a JSON array; a reply with no covers key keeps them.
    // Issue 561: the best attempt's failing set becomes the subset baseline; a strict shrink of the previous attempt's set is neutral; new workflow_decompose_v1_failing_set.js.
    // Issue 559 guard: the entry author reads the current entry with state.entries?.get.
    // Issue 562: fewer open findings and a subject's advance to a later gate stage are progress; gate findings carry their stage.
    // Issue 560: the decomposition entry author's absent-path rule matches the re-author's: absence means behaviour not implemented yet only for a path established to name a deciding file.
    // Issue 563: the decomposition entry author prompt states the check site PATH when the launcher passes it.
    // Issue 568: a skeleton re-author prompt names the previous candidate in a host-written context file (path and sha256), never inline.
    // Issue 569: the skeleton prompt states the obligation rule and the host-listed obligation ids, and its shape shows the record-binding fields.
    // Issue 571: the acceptance shape and covers rule name the REQ- requirement prefix.
    // Issue 572: the skeleton and body authors get the host-listed PRD constraint texts and their placement and keep rules.
    // Issue 574/575: constraint wording names table rows; the skeleton shape gains source_sections, the skeleton author gets the qualifying PRD section keys and texts, and the body author its frozen source_sections and their texts.
    // #584 P1-P6 integration: acceptance, skeleton and body author scripts changed.
    // #587: named Skeleton findings also reach their body author after re-freeze.
    // #593: a shape refusal no longer stops the acceptance author window.
    // #599: deliverable contract rule in the skeleton prompt.
    // Issue 601: body repair retains the best actionable candidate within one author episode; authors no longer write observations.
    assert_eq!(
        archon_workflow::workflow_scaffold_hash(
            crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE
        ),
        "24d8adaea4d87f8e75bb5e584c822dc4fc98ab6727be9ab8986912ed0080ee47"
    );
}
