use archon_workflow::acceptance_scratch::{ScratchPolicy, observe_commands};
use archon_workflow::acceptance_world::{FrozenCommandRef,AcceptanceCommandKind};
use archon_workflow::task_set_contract::{AcceptanceContract,content_digest};
use std::{path::Path,process::Command,collections::BTreeMap};
fn git(root:&Path,args:&[&str])->String {let o=Command::new("git").arg("-C").arg(root).args(args).output().unwrap();assert!(o.status.success(),"{}",String::from_utf8_lossy(&o.stderr));String::from_utf8(o.stdout).unwrap().trim().into()}
fn fixture(command:&str)->(tempfile::TempDir,ScratchPolicy,String,AcceptanceContract,Vec<FrozenCommandRef>) {
 let t=tempfile::tempdir().unwrap();let repo=t.path().join("repo");let project=t.path().join("project");std::fs::create_dir_all(&repo).unwrap();std::fs::create_dir_all(project.join("data")).unwrap();std::fs::create_dir_all(project.join("tasks")).unwrap();
 git(&repo,&["init","-q"]);git(&repo,&["config","user.email","test@example.invalid"]);git(&repo,&["config","user.name","test"]);std::fs::write(repo.join("input"),"source").unwrap();git(&repo,&["add","."]);git(&repo,&["commit","-qm","fixture"]);let commit=git(&repo,&["rev-parse","HEAD"]);
 std::fs::write(project.join("data/value"),"before").unwrap();
 let p=ScratchPolicy {repository:repo,project:project.clone(),task_root:project.join("tasks"),scratch_parent:t.path().join("scratch"),project_inputs:vec!["data".into()],combined:true,toolchain_path:"/usr/bin:/bin:/usr/sbin:/sbin".into(),environment:BTreeMap::new(),cargo_seed:None,timeout_secs:3,output_bytes:2048,scratch_bytes:16*1024*1024};
 let c:AcceptanceContract=serde_json::from_value(serde_json::json!({"schema_version":1,"prd":{"path":"p","digest":"d"},"gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},"acceptance":[{"id":"AC-X-001","criterion":"output correct","check":{"kind":"command","command":command,"cwd":"project_root"},"judgment":{"verdict":"accepted","counterexample":"incorrect output","reason":"checks output","host_call_id":"j"}}]})).unwrap();
 let refs=vec![FrozenCommandRef {acceptance_id:"AC-X-001".into(),kind:AcceptanceCommandKind::Command,chain_digest:"chain".into(),command_digest:content_digest(command.as_bytes())}];(t,p,commit,c,refs)
}
#[tokio::test]
async fn native_command_mutates_only_scratch_and_records_verified_cleanup() {
 let (t,p,commit,c,r)=fixture("printf after > data/value; test \"$(cat data/value)\" = after");
 let out=observe_commands(&p,&commit,&c,"chain",&r,&t.path().join("evidence")).await.unwrap();
 assert!(out.passed());assert_eq!(out.checks[0].exit_code,Some(0));assert!(out.live_roots_unchanged && out.teardown_verified);
 assert_eq!(std::fs::read_to_string(p.project.join("data/value")).unwrap(),"before");
 assert_eq!(std::fs::read_dir(&p.scratch_parent).unwrap().count(),0);
}
#[tokio::test]
async fn direct_live_write_voids_even_a_zero_exit() {
 let (t,p,commit,mut c,mut r)=fixture("test -f data/value");let cmd=format!("printf changed > '{}'; test -f data/value",p.project.join("data/value").display());
 if let archon_workflow::task_set_contract::AcceptanceCheck::Command {command,..}=&mut c.acceptance[0].check {*command=cmd.clone();}
 r[0].command_digest=content_digest(cmd.as_bytes());
 let out=observe_commands(&p,&commit,&c,"chain",&r,&t.path().join("evidence")).await.unwrap();assert!(!out.passed());assert!(!out.live_roots_unchanged);
}
#[tokio::test]
async fn timeout_and_output_flood_are_operational_and_cleaned() {
 for cmd in ["sleep 30; test -f input","while :; do printf flood; done"] {
  let (t,mut p,commit,c,r)=fixture(cmd);p.timeout_secs=1;
  let out=observe_commands(&p,&commit,&c,"chain",&r,&t.path().join("evidence")).await.unwrap();assert!(!out.passed());assert!(out.checks[0].operational_error.is_some());assert!(out.teardown_verified);
 }
}
#[tokio::test]
async fn authorization_failure_creates_no_scratch_and_runs_nothing() {
 let (t,p,commit,c,mut r)=fixture("printf after > data/value; test -f input");r[0].command_digest="wrong".into();
 assert!(observe_commands(&p,&commit,&c,"chain",&r,&t.path().join("evidence")).await.is_err());assert!(!p.scratch_parent.exists());
}
