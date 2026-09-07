//! Replay externally supplied evidence only in disposable Git state.
use super::*;

struct UnusedDispatch;
#[async_trait::async_trait]
impl WorkflowAgentDispatch for UnusedDispatch {
    fn fanout_parallelism(&self,_:Option<usize>)->usize{1}
    async fn run_call(&self,_:&str,_:Option<String>,_:&WorkflowV2CallExecution,_:&WorkflowV2AgentAdapter,
        _:Option<&WorkflowV2ResultStore>,_:Option<&WorkflowV2TaskUniverse>)->WorkflowResult<WorkflowV2Result>{
        panic!("apply replay must never dispatch an agent")
    }
}
fn git(root:&Path,args:&[&str])->String {
    let out=std::process::Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(out.status.success(),"{args:?}: {}",String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().trim().into()
}
#[test]
#[ignore="manual preserved patch replay; paths supplied by operator"]
fn preserved_patch_commits_through_production_apply_wrapper(){
    let source=PathBuf::from(std::env::var("WRITE_REPLAY_REPOSITORY").unwrap());
    let patch=PathBuf::from(std::env::var("WRITE_REPLAY_PATCH").unwrap());
    let baseline_id=std::env::var("WRITE_REPLAY_BASELINE").unwrap();
    let report=PathBuf::from(std::env::var("WRITE_REPLAY_REPORT").unwrap());
    let patch_bytes=std::fs::read(&patch).unwrap();
    let temp=tempfile::tempdir().unwrap();let repo=temp.path().join("repo");
    let clone=std::process::Command::new("git").args(["clone","--quiet","--no-hardlinks","--shared"])
        .arg(&source).arg(&repo).status().unwrap();assert!(clone.success());
    git(&repo,&["checkout","--detach",&baseline_id]);
    let names=std::process::Command::new("git").args(["apply","--numstat"]).arg(&patch).output().unwrap();assert!(names.status.success());
    let targets=String::from_utf8(names.stdout).unwrap().lines().map(|line|
        normalize_target(line.splitn(3,'\t').nth(2).unwrap(),&repo).unwrap()).collect::<Vec<_>>();
    let run_root=temp.path().join("run");let item="preserved-item";
    let plan=WritePlan {
        run_id:"replay".into(),stage_id:"preserved-wave".into(),item_id:item.into(),canonical_root:repo.clone(),
        isolated_root:temp.path().join("workspace"),target_files:targets,target_dir_scopes:vec![],
        target_files_source:TargetFilesSource::Item,read_context_files:vec![],verify_inputs:vec![],baseline_id:"git:HEAD".into(),
        workspace_boundary_required:true,resource_keys:Default::default(),
    };
    let cfg=WriteCoordinatorConfig::default();
    let baseline=capture_canonical_baseline(&repo,&plan,&[],&cfg).unwrap();
    let workspace=create_item_workspace(&repo,&plan,&baseline).unwrap();
    git(&plan.isolated_root,&["apply",patch.to_str().unwrap()]);
    let captured=capture_patch(&workspace,&plan.target_files,&baseline).unwrap();
    assert!(!captured.patch_bytes.is_empty());
    let manifest_path=persist_manifest(&run_root,"replay","preserved-wave",item,&captured,ManifestStatus::PendingApply).unwrap();
    let manifest:PatchManifest=serde_json::from_slice(&std::fs::read(manifest_path).unwrap()).unwrap();
    let store=WorkflowStore::project(&temp.path().join("project"));
    let v2=WorkflowV2ResultStore::new(run_root.join("v2"));
    let execution=WorkflowV2CallExecution{call:WorkflowV2HostCall{id:"preserved-wave".into(),method:WorkflowV2HostMethod::Fanout,
        write_mode:Some(WorkflowV2WriteMode::Worktree),options:Default::default()},input:serde_json::json!({}),depends_on:vec![]};
    let setup=WorktreeFanoutSetup{canonical_root:repo.clone(),cfg,run_root:run_root.clone()};
    let ctx=WorktreePlanRunContext{task:"replay preserved patch",target_repository_root:repo.to_str(),execution:&execution,
        adapter:WorkflowV2AgentAdapter::new(),dispatch:&UnusedDispatch,v2_store:&v2,store_for_control:&store,run_id:"replay",
        setup:&setup,semaphore:Arc::new(Semaphore::new(1)),active:Arc::new(AtomicUsize::new(0)),peak:Arc::new(AtomicUsize::new(0)),task_universe:None};
    let mut artifacts=WorktreeWaveArtifacts{manifests:vec![manifest],pre_hashes:BTreeMap::from([(item.into(),captured.pre_hashes.clone())]),..Default::default()};
    let gap=apply_worktree_wave(&ctx,0,&mut artifacts);
    let final_head=git(&repo,&["rev-parse","HEAD"]);
    let record=serde_json::json!({"baseline":baseline_id,"final_commit":final_head,"apply_gap":gap,
        "changed_files":captured.changed_files,"created_files":captured.created_files,"patch_bytes":captured.patch_bytes.len(),
        "commit_message":git(&repo,&["log","-1","--format=%s"]),"production_entry":"apply_worktree_wave","disposable":true});
    std::fs::write(report,serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    assert!(gap.is_none(),"{gap:?}");assert_ne!(final_head,baseline_id);
    for path in captured.changed_files.iter().chain(&captured.created_files) {
        assert_eq!(std::fs::read(repo.join(path)).unwrap(),std::fs::read(plan.isolated_root.join(path)).unwrap());
    }
    assert_eq!(std::fs::read(&patch).unwrap(),patch_bytes,"original evidence changed");
}
