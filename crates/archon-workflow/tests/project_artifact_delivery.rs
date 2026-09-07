use archon_workflow::*;
use archon_workflow::v2::call_data::{fanout_items_for_call,v2_agent_request};
use serde_json::json;
use std::path::PathBuf;
struct ArtifactAgent { project:PathBuf, write:bool, code:bool }
#[async_trait::async_trait]
impl WorkflowAgentDispatch for ArtifactAgent {
    fn fanout_parallelism(&self,_:Option<usize>)->usize{1}
    async fn run_call(&self,task:&str,root:Option<String>,e:&WorkflowV2CallExecution,a:&WorkflowV2AgentAdapter,
        store:Option<&WorkflowV2ResultStore>,universe:Option<&task_universe::WorkflowV2TaskUniverse>)->WorkflowResult<WorkflowV2Result> {
        if e.call.write_mode.is_none(){return Ok(WorkflowV2Result::accepted("scope unchanged"));}
        if self.write {
            std::fs::create_dir_all(self.project.join("reports")).unwrap();
            std::fs::write(self.project.join("reports/report.md"),"verified project report\n").unwrap();
        }
        if self.code {std::fs::write(PathBuf::from(root.as_ref().unwrap()).join("input"),"updated code").unwrap();}
        let mut req=v2_agent_request(task,root.clone(),e,universe);
        req.project_artifacts=project_artifact_context_from_v2_root(store.unwrap().root());
        req.project_artifacts.repository_root=root;
        req.project_artifacts.add_contract_artifact_paths(universe.unwrap(),&e.input["item"]);
        req.project_artifacts.add_artifact_requirements(&e.input);
        let mut changed=vec![json!({"path":"reports/report.md"})];
        if self.code {changed.push(json!({"path":"input"}));}
        let output=json!({"status":"accepted","summary":"created declared project report",
            "evidence":[{"kind":"implementation","summary":"project report written"}],
            "files_changed":changed,"commands_run":[{"kind":"test","command":"test -s reports/report.md","status":"succeeded","exit_code":0,"output_summary":"report checked"}],
            "task_coverage":[{"task_id":"TASK-001","status":"accepted","summary":"report delivered","evidence":[{"kind":"artifact","summary":"report exists"}]}],"data":{}});
        a.parse_agent_output(&req,&output.to_string()).map_err(|e|WorkflowError::StageFailed(format!("schema repair failed: {e}")))
    }
}
async fn exercise(write:bool){ exercise_mixed(write,false,false).await; }
async fn exercise_mixed(write:bool,mixed:bool,code:bool){
    let temp=tempfile::tempdir().unwrap();let repo=temp.path().join("repo");let project=temp.path().join("project");std::fs::create_dir(&repo).unwrap();
    let git=|args:&[&str]|{let o=std::process::Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();assert!(o.status.success());String::from_utf8(o.stdout).unwrap().trim().to_string()};
    git(&["init","-q"]);git(&["config","user.name","fixture"]);git(&["config","user.email","fixture@example.invalid"]);
    std::fs::write(repo.join("input"),"base").unwrap();git(&["add","input"]);git(&["commit","-qm","baseline"]);let before=git(&["rev-parse","HEAD"]);
    let universe:task_universe::WorkflowV2TaskUniverse=serde_json::from_value(json!({"schema_version":"v1","source_roots":[],"tasks":[{"canonical_task_id":"TASK-001","source_path":project.join("task.md"),"deliverable_contracts":[{"kind":"report","artifact_path":"reports/report.md"}]}]})).unwrap();
    let store=WorkflowStore::project(&project);let run=store.create_run(WorkflowSpec{schema:spec::WORKFLOW_SCHEMA.into(),name:"artifact-test".into(),task:"deliver report".into(),target_repository_root:Some(repo.display().to_string()),max_parallelism:1,max_agents:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
    let v2=WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let call=WorkflowV2HostCall{id:"artifact-wave".into(),method:WorkflowV2HostMethod::Fanout,write_mode:Some(WorkflowV2WriteMode::Worktree),options:WorkflowV2HostOptions{item_kind:Some("implementation".into()),target_files_from_item:true,..Default::default()}};
    let e=WorkflowV2CallExecution{call,input:json!({"source_data":[{"item_id":"report","canonical_task_ids":["TASK-001"],"target_files":if mixed {vec!["input"]}else{vec![]},"artifact_requirements":["reports/report.md"]}]}),depends_on:vec![]};
    let branches=fanout_items_for_call(&e,&v2).unwrap();
    let result=v2::write::run_write_capable_v2_fanout("deliver report",repo.to_str(),e,WorkflowV2AgentAdapter::new(),&ArtifactAgent{project:project.clone(),write,code},&v2,&store,&run.id,true,branches,Some(&universe),None).await.unwrap();
    if code && write {assert_ne!(git(&["rev-parse","HEAD"]),before);}
    else {assert_eq!(git(&["rev-parse","HEAD"]),before,"project artifacts must not be silently committed to repository");}
    if write {
        assert_eq!(result.status,WorkflowV2Status::Accepted,"{result:#?}");
        let outcome=v2.load_branch_outcomes().unwrap().into_iter().find(|o|o.result.as_ref().is_some_and(|r|r.data["canonical_task_ids"]==json!(["TASK-001"]))).expect("host task outcome").result.unwrap();
        assert_eq!(outcome.data["delivery"]["kind"],"project_artifact");
        assert_eq!(outcome.data["delivery"]["repository_changed"],code);
        assert_eq!(outcome.data["delivery"]["changed_artifact_paths"],json!(["reports/report.md"]));
    }else{assert_ne!(result.status,WorkflowV2Status::Accepted,"missing report accepted as empty repo patch");}
}
#[tokio::test]
async fn external_project_report_is_verified_and_distinguished_from_repo_patch(){exercise(true).await;}
#[tokio::test]
async fn missing_external_report_cannot_pass_via_empty_repository_patch(){exercise(false).await;}

#[tokio::test]
async fn mixed_task_records_artifact_delivery_alongside_repository_patch(){exercise_mixed(true,true,true).await;}
#[tokio::test]
async fn mixed_task_artifact_only_change_has_delivery_evidence(){exercise_mixed(true,true,false).await;}
#[tokio::test]
async fn mixed_task_missing_artifact_still_fails(){exercise_mixed(false,true,true).await;}
