//! Issue 331: a run's start warns -- an event and a log line, never a
//! refusal -- about what its checks run that the configured toolchain path
//! does not resolve.

use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use archon_workflow::WorkflowStore;
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

use super::warn;
use crate::command::workflow_task_set::republish::test_fixture::frozen_set;

struct Lines(Arc<Mutex<Vec<String>>>);

#[async_trait::async_trait]
impl archon_workflow::WorkflowUiSink for Lines {
    async fn emit(
        &self,
        event: archon_workflow::WorkflowUiEvent,
    ) -> archon_workflow::WorkflowUiResult {
        if let archon_workflow::WorkflowUiEvent::Text(text) = event {
            self.0.lock().unwrap().push(text);
        }
        Ok(())
    }
}

fn git(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A run of a frozen set whose checks are `checks`, under a scratch policy
/// whose toolchain path is a bin directory holding `tools` and the system's
/// directories; returns (store, run id, toolchain path, kept dirs).
fn launched(
    checks: &[(&str, &str, bool)],
    tools: &[&str],
) -> (WorkflowStore, String, String, Vec<tempfile::TempDir>) {
    let set = frozen_set(checks);
    let outside = tempfile::tempdir().unwrap();
    let repo = outside.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &[
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "base",
        ],
    );
    let bin = outside.path().canonicalize().unwrap().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for tool in tools {
        let file = bin.join(tool);
        let script = match *tool {
            "archon331tool" => {
                "#!/bin/sh\n[ \"$1\" = --list ] && printf 'Commands:\\n    build    Build\\n'\nexit 0\n"
            }
            _ => "#!/bin/sh\nexit 0\n",
        };
        std::fs::write(&file, script).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let toolchain = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display());
    std::fs::create_dir_all(set.project.path().join(".archon")).unwrap();
    std::fs::write(
        set.project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path={toolchain:?}\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
            repo.display().to_string(),
            outside.path().join("scratch").display().to_string(),
        ),
    )
    .unwrap();
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".into(),
        source_roots: vec![set.tasks.display().to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-F-001".into(),
            source_path: set.tasks.join("TASK-F-001.md").display().to_string(),
            ..WorkflowV2TaskUniverseTask::default()
        }],
    };
    let plan = super::super::super::WorkflowScriptPlan::generated(
        "implement decomposed tasks",
        "export default async function workflow(w) { await w.checkpoint(\"done\", {}); }",
        Vec::new(),
        Some(universe),
        archon_core::config::GeneratedWorkflowConfig::default(),
        &archon_core::config::LearningConfig::default(),
    )
    .unwrap();
    let store = WorkflowStore::project(set.project.path());
    let run = store.create_run(plan.approval_metadata_spec()).unwrap();
    super::super::super::save_generated_v2_metadata(&store, &run.id, &plan, true).unwrap();
    (store, run.id, toolchain, vec![set.project, outside])
}

async fn warned(store: &WorkflowStore, run_id: &str) -> (Vec<serde_json::Value>, Vec<String>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink: archon_workflow::SharedWorkflowUiSink = Arc::new(Lines(lines.clone()));
    warn(store, run_id, &sink).await;
    let events = std::fs::read_to_string(store.events_path(run_id)).unwrap_or_default();
    let events = (events.lines())
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|event| event["detail"]["event"] == "toolchain_unresolved")
        .collect();
    let lines = lines.lock().unwrap().clone();
    (events, lines)
}

#[tokio::test]
async fn a_run_start_warns_about_commands_its_toolchain_path_does_not_resolve() {
    let checks = [
        ("AC-1", "archon331tool lint --all", true),
        ("AC-2", "archon331tool build && archon331tool other", true),
        ("AC-3", "archon-issue-331-absent --verify", true),
        ("AC-4", "archon-issue-331-refuted", false),
    ];
    let (store, run_id, toolchain, _kept) =
        launched(&checks, &["archon331tool", "archon331tool-other"]);
    let (events, lines) = warned(&store, &run_id).await;
    assert_eq!(events.len(), 1, "{events:?}");
    let detail = &events[0]["detail"];
    assert_eq!(detail["toolchain_path"], toolchain.as_str());
    assert_eq!(
        detail["checks"],
        serde_json::json!({
            "AC-1": ["`archon331tool lint` (not built into `archon331tool`, and no `archon331tool-lint` on the path)"],
            "AC-3": ["`archon-issue-331-absent` (not on the path)"],
        }),
        "a resolved built-in and plugin, and an unaccepted check, are not named"
    );
    assert_eq!(events[0]["kind"], "started", "{:?}", events[0]);
    assert!(
        lines.len() == 1 && lines[0].starts_with("Warning: 2 acceptance check(s)"),
        "{lines:?}"
    );
    assert!(lines[0].contains(&toolchain), "{lines:?}");
}

#[tokio::test]
async fn a_run_start_whose_checks_all_resolve_says_nothing() {
    let checks = [("AC-1", "archon331tool build && test -f x", true)];
    let (store, run_id, _, _kept) = launched(&checks, &["archon331tool", "archon331tool-other"]);
    let (events, lines) = warned(&store, &run_id).await;
    assert!(
        events.is_empty() && lines.is_empty(),
        "{events:?} {lines:?}"
    );
}
