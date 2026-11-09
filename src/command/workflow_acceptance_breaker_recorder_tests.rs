//! Live recording is opt-in; this module never runs in the offline gates.
use super::*;
use archon_workflow::base_snapshot::{SnapshotEntry, snapshot_deliverables};
use archon_workflow::llm_client_port::{
    WorkflowAgentOutcome, WorkflowLlmClientFactory, WorkflowLlmClientRequest,
};
use archon_workflow::repository_record::{RepositoryRecordV1, RepositoryTree};
use std::path::Path;
#[path = "workflow_acceptance_breaker_record_inputs.rs"]
mod inputs;
fn save(
    dir: &Path,
    name: &str,
    client: &dyn WorkflowLlmClient,
    model: &str,
    message: &str,
    reply: &WorkflowAgentOutcome,
    sources: &mut Vec<serde_json::Value>,
) {
    if let Some(source) = sources.iter_mut().find(|s| s["case"] == name) {
        let previous = format!("{name}_round3");
        for suffix in ["txt", "transcript.json"] {
            std::fs::rename(
                dir.join(format!("{name}.{suffix}")),
                dir.join(format!("{previous}.{suffix}")),
            )
            .unwrap();
        }
        source["case"] = previous.clone().into();
        source["transcript_path"] = format!("{previous}.transcript.json").into();
    }
    std::fs::write(dir.join(format!("{name}.txt")), &reply.content).unwrap();
    let transcript = format!("{name}.transcript.json");
    std::fs::write(dir.join(&transcript),serde_json::to_vec(&serde_json::json!({"prompt":message,"reply":reply.content,"stop_reason":reply.stop_reason,"tool_calls":reply.tool_uses.iter().map(|u|serde_json::json!({"name":u.tool_name,"input":u.input,"output":u.output})).collect::<Vec<_>>()})).unwrap()).unwrap();
    sources.push(serde_json::json!({"case":name,"provider_id":client.provider_id(),"resolved_model":client.resolve_model_alias(model),"request_identity":client.request_identity(),"date":chrono::Utc::now().to_rfc3339(),"stop_reason":reply.stop_reason,"tool_call_count":reply.tool_uses.len(),"transcript_path":transcript}));
    std::fs::write(dir.join("README.md"),"# Breaker reply corpus\n\nRecording is in progress. sources.json lists completed calls and their transcripts. Files not yet replaced may remain PENDING-RECORDING.\n").unwrap();
    std::fs::write(
        dir.join("sources.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"status":"recording_in_progress","sources":sources}),
        )
        .unwrap(),
    )
    .unwrap();
}
fn record_parse(
    dir: &Path,
    sources: &mut [serde_json::Value],
    parsed: &Result<prompt::Parsed, String>,
) {
    let source = sources.last_mut().expect("recorded source");
    let (mutants, reasons) = match parsed {
        Ok(p) => (p.mutants.as_slice(), p.refusals.clone()),
        Err(e) => (&[][..], vec![e.clone()]),
    };
    source["parser_refusals"] = serde_json::json!(reasons);
    source["valid_behaviour"] = mutants
        .iter()
        .filter(|(m, _)| m.kind == MutantKind::Behaviour)
        .count()
        .into();
    source["valid_body_gut"] = mutants
        .iter()
        .filter(|(m, _)| m.kind == MutantKind::BodyGut)
        .count()
        .into();
    std::fs::write(
        dir.join("sources.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"status":"recording_in_progress","sources":sources}),
        )
        .unwrap(),
    )
    .unwrap();
}
#[tokio::test]
#[ignore = "requires explicit opt-in and a configured real provider"]
async fn record_breaker_reply_corpus() {
    assert_eq!(
        std::env::var("ARCHON_RECORD_BREAKER_CORPUS").as_deref(),
        Ok("1"),
        "recording requires ARCHON_RECORD_BREAKER_CORPUS=1"
    );
    let project = PathBuf::from(
        std::env::var_os("ARCHON_BREAKER_CORPUS_PROJECT")
            .expect("set ARCHON_BREAKER_CORPUS_PROJECT to the configured project root"),
    );
    let path = project.join(".archon/config.toml");
    assert!(path.is_file());
    let config = archon_core::config::load_config_from(path).unwrap();
    let env = archon_core::env_vars::load_env_vars();
    let factory =
        crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
            &config, &env,
        )
        .without_project_tools();
    let client = factory
        .build_client(WorkflowLlmClientRequest {
            cwd: project.clone(),
            origin: "breaker-corpus".into(),
            session_id: uuid::Uuid::new_v4().to_string(),
            read_roots: vec![],
        })
        .await
        .unwrap();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/breaker_reply_corpus");
    std::fs::create_dir_all(&dir).unwrap();
    let f = super::tests::fixture(inputs::CHECK);
    let view = materialise_view(
        f.parent.path(),
        f.source.path(),
        f.snapshot(),
        &[(
            "value".into(),
            archon_workflow::base_snapshot::SnapshotKind::Blob,
        )],
        &[],
    )
    .unwrap();
    let id = "REQ-VALUE-001";
    let text = "It keeps the good value for every input.";
    let judges_only = std::env::var("ARCHON_BREAKER_CORPUS_JUDGES_ONLY").as_deref() == Ok("1");
    let mut sources = if dir.join("sources.json").exists() {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("sources.json")).unwrap()).unwrap();
        value["sources"].as_array().unwrap().clone()
    } else {
        vec![]
    };
    let mut refused = vec![];
    let model = client.model_for_role("breaker", JUDGE_MODEL_ALIAS);
    if !judges_only {
        inputs::record_pair(
            &dir,
            "breaker",
            client.as_ref(),
            &model,
            id,
            text,
            &view,
            &mut sources,
            &mut refused,
        )
        .await;
    }
    view.remove().unwrap();
    // R18 fixture text only; never reads any owner's PRD directory.
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commit = super::super::tests::git(repository, &["rev-parse", "HEAD"]);
    let tree = RepositoryTree::load(&RepositoryRecordV1 {
        schema_version: 1,
        repository_root: repository.display().to_string(),
        base_commit: commit,
        decomposition_run_id: "corpus".into(),
        recorded_at: "corpus".into(),
    })
    .unwrap();
    let skeleton: archon_workflow::task_skeleton::TaskSkeleton = serde_json::from_str(
        include_str!("../../tests/fixtures/r18_plan/task-skeleton.json"),
    )
    .unwrap();
    let prd = include_str!("../../tests/fixtures/r18_plan/prd.md");
    let mut selected = None;
    for task in &skeleton.tasks {
        for d in &task.deliverable_contracts {
            let shared = skeleton
                .tasks
                .iter()
                .filter(|other| other.task_id != task.task_id)
                .flat_map(|other| &other.deliverable_contracts)
                .any(|other| {
                    d.artifact_path == other.artifact_path
                        || d.artifact_path
                            .starts_with(&format!("{}/", other.artifact_path))
                        || other
                            .artifact_path
                            .starts_with(&format!("{}/", d.artifact_path))
                });
            if shared {
                continue;
            }
            let snapshot =
                snapshot_deliverables(&tree, repository, [d.artifact_path.as_str()]).unwrap();
            if let Some(SnapshotEntry::Tracked { kind, .. }) =
                snapshot.entries.get(&d.artifact_path)
                && let Some((id, text)) = task
                    .implements
                    .iter()
                    .find_map(|id| rule::requirement_text(prd, id).map(|text| (id.clone(), text)))
            {
                selected = Some((id, text, d.artifact_path.clone(), *kind, snapshot));
                break;
            }
        }
        if selected.is_some() {
            break;
        }
    }
    let (rid, rtext, key, kind, snapshot) =
        selected.expect("R18 fixture needs a recorded deliverable");
    let rview =
        materialise_view(f.parent.path(), repository, &snapshot, &[(key, kind)], &[]).unwrap();
    if !judges_only {
        inputs::record_pair(
            &dir,
            "breaker_r18",
            client.as_ref(),
            &model,
            &rid,
            &rtext,
            &rview,
            &mut sources,
            &mut refused,
        )
        .await;
    }
    rview.remove().unwrap();
    for (name, role, message, observation) in inputs::messages(&f, id, text).await {
        let retry = std::env::var("ARCHON_BREAKER_CORPUS_JUDGES_RETRY").as_deref() == Ok("1");
        if retry && matches!(name, "relevant" | "equivalent") {
            continue;
        }
        if retry {
            let previous = format!("{name}_attempt1");
            std::fs::copy(
                dir.join(format!("{name}.txt")),
                dir.join(format!("{previous}.txt")),
            )
            .unwrap();
            std::fs::copy(
                dir.join(format!("{name}.transcript.json")),
                dir.join(format!("{previous}.transcript.json")),
            )
            .unwrap();
            if let Some(source) = sources.iter_mut().find(|s| s["case"] == name) {
                source["case"] = previous.clone().into();
                source["transcript_path"] = format!("{previous}.transcript.json").into();
            }
        }

        let model = client.model_for_role(
            role,
            if role == "breaker_equivalence_second" {
                JUDGE_MODEL_ALIAS
            } else {
                "opus"
            },
        );
        let reply = client
            .send_message_with_temperature(
                vec![serde_json::json!({"role":"user","content":message})],
                vec![],
                vec![],
                &model,
                0.0,
            )
            .await
            .unwrap();
        save(
            &dir,
            &format!("{name}_round5"),
            client.as_ref(),
            &model,
            &message,
            &reply,
            &mut sources,
        );
        sources.last_mut().expect("recorded source")["observation"] = observation;
        if let Err(e) = prompt::object(&reply.content) {
            refused.push(format!("{name}: {e}"));
        }
    }
    std::fs::write(dir.join("README.md"),"# Breaker reply corpus\n\nRecorded by record_breaker_reply_corpus from the configured provider. sources.json lists provenance, transcripts, and parser refusals. Tests derive CRLF, truncated, and inner-fence variants in memory from these files. Read these replies before freezing the grammar.\n").unwrap();
    let success = inputs::review_recorded(&dir, &f, &mut sources, &mut refused);
    for r in &refused {
        eprintln!("parser refusal: {r}");
    }
    std::fs::write(
        dir.join("sources.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"status":"recorded","sources":sources,"parser_refusals":refused}),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        success,
        "recorded pairs did not meet the valid behaviour/body-gut bar; literal replies and parser reasons are in the corpus"
    );
}

#[test]
fn recorder_persists_literal_parse_refusal_before_next_call() {
    let reply = include_str!("../../tests/fixtures/breaker_reply_corpus/breaker_r18_attempt3.txt");
    let reason = prompt::object(reply).unwrap_err();
    let parsed = Err(reason.clone());
    let dir = tempfile::tempdir().unwrap();
    let mut sources = vec![serde_json::json!({"case":"recorded_truncated_reply"})];
    record_parse(dir.path(), &mut sources, &parsed);
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("sources.json")).unwrap()).unwrap();
    assert_eq!(
        stored["sources"][0]["parser_refusals"],
        serde_json::json!([reason])
    );
    assert_eq!(stored["sources"][0]["valid_behaviour"], 0);
    assert_eq!(stored["sources"][0]["valid_body_gut"], 0);
}
