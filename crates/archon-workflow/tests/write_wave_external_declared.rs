//! Issue-226 through the production write-wave seam: a write branch whose
//! call declares an artifact outside both the project and the repository.
//!
//! Inside a directory the run's policy file lists, the branch writes its own
//! staged copy and the host lands it through the audited project-input
//! landing: a missing root is created there and recorded, a landing that
//! fails part way (a read-only directory: a clear, logged permission error,
//! never a silent skip) undoes everything it wrote, the directories it made
//! included. Anywhere else -- an unlisted directory, a link out of a listed
//! one, a `..` -- nothing is written, and the run's ledger refuses a grant
//! of it naming the root and the policy key. With no list, the run behaves
//! as before the key existed. Every check reads the files back.
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archon_workflow::task_scope_amendment::{
    ScopeAmendment, ScopeAmendmentRequest, ScopeGrantKind, ScopeGrantRoot, amend_task_scope,
};
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::*;
use serde_json::{Value, json};
use support::{COPY, Edits, Fixture, git};

const KEY: &str = "workflow.acceptance_execution.external_data_roots";
const BEFORE: &str = "{\"close\": 0}\n";
const AFTER: &str = "{\"close\": 101}\n";

struct World {
    _temp: tempfile::TempDir,
    f: Fixture,
    allowed: PathBuf,
    elsewhere: PathBuf,
}

/// A run whose policy file lists `allowed` (or nothing, `listed` false).
fn world(listed: bool) -> World {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let (allowed, elsewhere) = (base.join("allowed"), base.join("elsewhere"));
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    let f = Fixture::new();
    std::fs::write(f.repo.join(".gitignore"), ".archon/*\n").unwrap();
    git(&f.repo, &["add", ".gitignore"]);
    git(&f.repo, &["commit", "-qm", "ignore project state"]);
    let project = PathBuf::from(
        project_artifact_context_from_v2_root(f.v2.root())
            .project_root
            .unwrap(),
    )
    .canonicalize()
    .unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    let policy = json!({
        "repository": f.repo.canonicalize().unwrap(), "project": project,
        "task_root": project.join("tasks"), "scratch_parent": base.join("scratch"),
        "project_inputs": [], "project_input_excludes": [],
        "combined": true, "toolchain_path": "/usr/bin:/bin", "environment": {},
        "environment_allowlist": [], "cargo_seed": null, "timeout_secs": 60,
        "output_bytes": 4096, "scratch_bytes": 1u64 << 30,
    });
    let mut native =
        json!({"policy": policy, "source_commit": git(&f.repo, &["rev-parse", "HEAD"])});
    if listed {
        native["external_data_roots"] = json!([allowed]);
    }
    let metadata = json!({"observer_snapshot": {"native_execution": native}});
    let path = f.store.run_dir(&f.run).join("v2/generated-metadata.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    World {
        _temp: temp,
        f,
        allowed,
        elsewhere,
    }
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// The branch's own change, and `content` at its copy of each of `copies`.
fn edits(copies: &[&Path], content: &'static str) -> Edits {
    let mut files = vec![("owned.txt", "implemented\n")];
    files.extend((copies.iter()).map(|path| (leak(format!("{COPY}{}", path.display())), content)));
    Edits {
        files,
        report: vec!["owned.txt"],
        via_adapter: false,
    }
}

/// One write branch whose call declares `artifacts`, doing `edits`.
async fn wave(f: &Fixture, artifacts: &[&Path], edits: Edits) -> (WorkflowV2Result, Vec<String>) {
    let call = WorkflowV2HostCall {
        id: "impl".into(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions {
            item_kind: Some("implementation".into()),
            task: Some("Implement the item now.".into()),
            target_files_from_item: true,
            ..Default::default()
        },
    };
    let mut branch = call.clone();
    branch.id = "impl-0".into();
    branch.method = WorkflowV2HostMethod::Implementation;
    branch.options.target_files = vec!["owned.txt".into()];
    let artifacts: Vec<String> = artifacts.iter().map(|p| p.display().to_string()).collect();
    let item = WorkflowV2FanoutItem::read_only(
        "impl-0".to_string(),
        "coder",
        branch,
        json!({"item": {"item_id": "impl-0", "canonical_task_ids": ["TASK-001"],
            "target_files": ["owned.txt"], "work_type": "implementation",
            "artifact_requirements": artifacts}}),
    );
    f.wave_on(&f.v2, call, vec![(item, edits)], (None, &[], &[]), false)
        .await
}

/// The landing log's decided lines (no intents).
fn landings(f: &Fixture) -> Vec<Value> {
    let log = f
        .store
        .run_dir(&f.run)
        .join("write-coordination/project-inputs.jsonl");
    (std::fs::read_to_string(log).unwrap_or_default().lines())
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| line["outcome"] != "intent")
        .collect()
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// The run's ledger asked to grant TASK-001 `path` as external data, the
/// task declaring `declared`: `Ok` when granted, else the logged refusal.
fn ask_external(f: &Fixture, path: &Path, declared: &Path) -> Result<(), String> {
    let run_root = f.store.run_dir(&f.run);
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-001".into(),
            source_path: "tasks/TASK-001.md".into(),
            artifact_requirements: vec![declared.display().to_string()],
            ..Default::default()
        }],
    };
    let path = path.display().to_string();
    let outcome = amend_task_scope(ScopeAmendmentRequest {
        run_root: &run_root,
        universe: &universe,
        repository_root: &f.repo,
        grants: vec![ScopeAmendment {
            task_id: "TASK-001".into(),
            path: path.clone(),
            kind: ScopeGrantKind::DeliverableRoot,
            root: ScopeGrantRoot::External,
            shared_with: BTreeSet::new(),
            evidence: "issue-226 test".into(),
        }],
        trigger: "issue-226 test",
    })
    .unwrap();
    let Some((_, why)) = outcome.refused.first() else {
        return Ok(());
    };
    let log = read(&run_root.join("v2/scope-amendments.jsonl")).unwrap();
    assert!(log.contains(&path), "the refusal is logged: {log}");
    Err(why.clone())
}

/// What a refused declaration leaves: nothing landed, nothing written, and
/// the prompt never offers a copy of it.
fn nothing_landed(result: &WorkflowV2Result, prompts: &[String], f: &Fixture, path: &Path) {
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert!(landings(f).is_empty(), "{:?}", landings(f));
    assert!(
        !prompts[0].contains(&format!("(lands at {})", path.display())),
        "{}",
        prompts[0]
    );
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
}

#[tokio::test]
async fn a_missing_root_under_an_allowlisted_directory_is_created_by_the_landing() {
    let w = world(true);
    let out = w.allowed.join("fresh/deep/out.json");
    let (result, prompts) = wave(&w.f, &[&out], edits(&[&out], AFTER)).await;
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert!(
        prompts[0].contains(&format!("(lands at {})", out.display())),
        "{}",
        prompts[0]
    );
    assert_eq!(read(&out).as_deref(), Some(AFTER));
    let log = landings(&w.f);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(log[0]["outcome"], json!("applied"));
    assert_eq!(log[0]["path"], json!(out));
    assert_eq!(log[0]["before"], json!("absent"));
    let fresh = w.allowed.join("fresh");
    assert_eq!(log[0]["created_dirs"], json!([fresh, fresh.join("deep")]));
}

#[cfg(unix)]
#[tokio::test]
async fn a_read_only_directory_fails_the_landing_clearly_and_undo_removes_what_it_made() {
    use std::os::unix::fs::PermissionsExt;
    let w = world(true);
    let out = w.allowed.join("fresh/deep/out.json");
    let locked = w.allowed.join("ro");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let blocked = locked.join("out.json");
    let (result, _) = wave(&w.f, &[&out, &blocked], edits(&[&out, &blocked], AFTER)).await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Never a silent skip: the branch is told, and the log says why.
    let gap = (result.residual_gaps.iter())
        .chain(w.f.branch_result("impl", "impl-0").residual_gaps.iter())
        .find(|gap| gap.id.starts_with("project_inputs_refused_"))
        .cloned()
        .unwrap_or_else(|| panic!("{result:#?}"));
    for text in ["PermissionDenied", &blocked.display().to_string()] {
        assert!(gap.description.contains(text), "{text}: {gap:#?}");
    }
    let log = landings(&w.f);
    assert!(!log.is_empty() && log.iter().all(|line| line["outcome"] == "refused"));
    assert!(
        (log.iter()).all(|line| line["reason"]
            .as_str()
            .unwrap()
            .contains("PermissionDenied")),
        "{log:#?}"
    );
    // All or nothing: the file it had written, and the root it made, are gone.
    assert!(!blocked.exists() && !out.exists());
    assert!(!w.allowed.join("fresh").exists(), "{log:#?}");
}

#[tokio::test]
async fn a_root_outside_the_allowlist_is_refused_naming_the_root_and_the_key() {
    let w = world(true);
    let out = w.elsewhere.join("lake/out.json");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::write(&out, BEFORE).unwrap();
    let (result, prompts) = wave(&w.f, &[&out], edits(&[], AFTER)).await;
    nothing_landed(&result, &prompts, &w.f, &out);
    assert_eq!(read(&out).as_deref(), Some(BEFORE));
    let why = ask_external(&w.f, &out, &out).unwrap_err();
    assert!(why.contains(KEY), "{why}");
    assert!(
        why.contains(&w.elsewhere.join("lake").display().to_string()),
        "{why}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_inside_an_allowlisted_directory_pointing_out_is_refused() {
    let w = world(true);
    let secret = w.elsewhere.join("secret/keys.json");
    std::fs::create_dir_all(secret.parent().unwrap()).unwrap();
    std::fs::write(&secret, BEFORE).unwrap();
    std::fs::create_dir_all(w.allowed.join("lake")).unwrap();
    std::os::unix::fs::symlink(w.elsewhere.join("secret"), w.allowed.join("lake/vault")).unwrap();
    let through = w.allowed.join("lake/vault/keys.json");
    let (result, prompts) = wave(&w.f, &[&through], edits(&[], AFTER)).await;
    nothing_landed(&result, &prompts, &w.f, &through);
    assert_eq!(read(&secret).as_deref(), Some(BEFORE));
    let index = w.allowed.join("lake/index.json");
    for asked in [through.clone(), secret.clone()] {
        let why = ask_external(&w.f, &asked, &index).unwrap_err();
        assert!(why.contains(KEY), "{}: {why}", asked.display());
    }
}

#[tokio::test]
async fn a_dot_dot_declaration_is_refused() {
    let w = world(true);
    let target = w.elsewhere.join("x/out.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, BEFORE).unwrap();
    std::fs::create_dir_all(w.allowed.join("lake")).unwrap();
    let climbed = w.allowed.join("lake/../../elsewhere/x/out.json");
    let (result, prompts) = wave(&w.f, &[&climbed], edits(&[], AFTER)).await;
    nothing_landed(&result, &prompts, &w.f, &climbed);
    assert_eq!(read(&target).as_deref(), Some(BEFORE));
    let why = ask_external(&w.f, &climbed, &climbed).unwrap_err();
    assert!(why.contains("`..`") && why.contains(KEY), "{why}");
}

#[tokio::test]
async fn with_an_empty_allowlist_an_external_declaration_is_ignored_as_before() {
    let w = world(false);
    let out = w.allowed.join("lake/out.json");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    std::fs::write(&out, BEFORE).unwrap();
    let (result, prompts) = wave(&w.f, &[&out], edits(&[], AFTER)).await;
    nothing_landed(&result, &prompts, &w.f, &out);
    assert!(
        !prompts[0].contains("Resolved Project Artifact Paths"),
        "never a declared artifact of the branch, as before: {}",
        prompts[0]
    );
    assert_eq!(read(&out).as_deref(), Some(BEFORE));
    let seed = (w.f.store.run_dir(&w.f.run))
        .join("write-coordination/stages/impl/project-inputs/impl-0/seed.json");
    assert!(!seed.exists(), "nothing seeded for it");
    let why = ask_external(&w.f, &out, &out).unwrap_err();
    assert!(why.contains(KEY), "{why}");
}
