use super::*;

#[tokio::test]
async fn remediation_plateau_pauses_resumably_with_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec.clone()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let (sink, _rx) = default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        Arc::new(PanicLlm),
        sink,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "stall".into(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2.clone(),
        store.clone(),
        run.id.clone(),
        true,
        None,
        None,
    );
    // Execute the production remediation loop against two unsuccessful
    // branch attempts, then the live host's actual checkpoint/pause path.
    let source = include_str!("../../crates/archon-workflow/src/v2/script/v3_prim_remediate.js");
    let script = format!(
        r#"async function workflow(w) {{
      const withCompletionBlocked = (_, blocked) => blocked;
      const stringList = (v) => Array.isArray(v) ? v : [];
      const slug = (v) => v;
      const requestRemediationPlan = async () => ({{}});
      const planUnits = (all) => ({{units: [{{key:'T',taskIds:['T'],targetFiles:['a.rs'],own:all}}],unassigned:[],checks:new Set()}});
      const remediationCycle = async (_, __, ___, ctx) => {{
        for (let n=0; n<ctx.maxRounds; n++) await w.checkpoint(`unsuccessful-${{n}}`, {{summary:'no progress; partial work captured'}});
        return {{closed:[], reasons:{{F:'still failing'}}, verified:false, skippedForNoPatch:2,
          fix:{{status:'needs_review',data:{{branch_no_progress_stop:true,partial_work:{{patch_path:'saved.patch'}}}}}}}};
      }};
      const verbatimEvidence = (v) => v;
      {source}
      await remediateFindings([{{finding_id:'F'}}]);
      await w.checkpoint('after-stall');
    }}"#
    );
    let error = runner
        .run(&script)
        .await
        .expect_err("a plateau pauses the run");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
    assert!(
        std::fs::read_to_string(store.events_path(&run.id))
            .unwrap()
            .contains("saved.patch")
    );
    assert!(v2.load_call_record("after-stall").unwrap().is_none());
    archon_workflow::LifecycleController::new(store.clone())
        .apply(&run.id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    assert_ne!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
}
