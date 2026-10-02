//! Batch G2 (G2-2) through the production write-wave seam: a write branch
//! never writes the live project root. Its declared project artifacts are
//! written in its own copy -- in its worktree when the repository ignores
//! the path, in its staging directory when the path there is the
//! repository's -- and the host lands them through the audited project-data
//! landing, which logs each decision and keeps what it replaced. A direct
//! write to the live project root from the branch's shell is refused by the
//! OS write boundary (macOS, Linux), and even where it is not, the landing
//! refuses to write over a project copy that changed after it was seeded.
//!
//! Before Batch G2 each declared artifact was stamped writable where it
//! lives in the project root: the branch's shell wrote it there directly,
//! with no baseline, no log and no kept copy, and the tripwire exempted it.
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::path::{Path, PathBuf};

use archon_workflow::*;
use serde_json::json;
use support::{BASH, COPY, Edits, Fixture, git};

const REPORT: &str = "docs/reports/audit.md";
const SUMMARY: &str = "reports/summary.json";

fn project_root(f: &Fixture) -> PathBuf {
    PathBuf::from(
        project_artifact_context_from_v2_root(f.v2.root())
            .project_root
            .expect("the run has a project root"),
    )
    .canonicalize()
    .map(archon_shell::paths::plain)
    .unwrap()
}

/// The repository ignores `docs/` and project state, as the live target
/// does; the project holds last round's report; the run recorded an
/// acceptance policy (inputs elsewhere) at launch.
fn fixture(scratch: &Path) -> Fixture {
    let f = Fixture::new();
    std::fs::write(f.repo.join(".gitignore"), ".archon/*\ndocs/*\n").unwrap();
    git(&f.repo, &["add", ".gitignore"]);
    git(&f.repo, &["commit", "-qm", "ignore project state and docs"]);
    let project = project_root(&f);
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    std::fs::create_dir_all(project.join(".archon/lab/data")).unwrap();
    std::fs::create_dir_all(project.join("docs/reports")).unwrap();
    std::fs::write(project.join(REPORT), "old report\n").unwrap();
    let policy = json!({
        "repository": f.repo.canonicalize().map(archon_shell::paths::plain).unwrap(), "project": project,
        "task_root": project.join("tasks"), "scratch_parent": scratch,
        "project_inputs": [".archon/lab/data"], "project_input_excludes": [],
        "combined": true, "toolchain_path": support::toolchain_path(), "environment": {},
        "environment_allowlist": [], "cargo_seed": null, "timeout_secs": 60,
        "output_bytes": 4096, "scratch_bytes": 1u64 << 30,
    });
    let metadata = json!({"observer_snapshot": {"native_execution": {
        "policy": policy, "source_commit": git(&f.repo, &["rev-parse", "HEAD"])}}});
    let path = f.store.run_dir(&f.run).join("v2/generated-metadata.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    f
}

fn landings(f: &Fixture) -> Vec<serde_json::Value> {
    let log = f
        .store
        .run_dir(&f.run)
        .join("write-coordination/project-inputs.jsonl");
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|line| line["outcome"] != "intent")
        .collect()
}

/// One write branch whose call declares `artifacts`, doing `edits`.
async fn wave(f: &Fixture, artifacts: &[&str], edits: Edits) -> (WorkflowV2Result, Vec<String>) {
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

/// The live shape (a declared report under an ignored `docs/`): the branch
/// writes its worktree copy, its direct write to the live project root is
/// refused, and the host lands the copy through the audited landing.
#[tokio::test]
async fn a_declared_artifact_lands_through_the_audited_landing_never_directly() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp.path().join("scratch"));
    let project = project_root(&f);
    let live = project.join(REPORT);
    let direct = format!("{BASH}printf 'hacked\n' > {}", support::shell_path(&live));
    let edits = Edits {
        files: vec![
            ("owned.txt", "implemented\n"),
            ("", Box::leak(direct.into_boxed_str())),
            (
                "",
                "\u{0}bash:mkdir -p docs/reports && printf 'new report\\n' > docs/reports/audit.md",
            ),
        ],
        report: vec!["owned.txt"],
        via_adapter: true,
    };
    // A declared path no landing may write (the project's configuration):
    // the agent is told so, and it is a defect of the declaration recorded
    // for review -- never the branch's failure.
    let (result, prompts) = wave(&f, &[REPORT, "config/app.toml"], edits).await;
    if !archon_tools::bash::shell_write_boundary_available() {
        // Issue-227: a host with no OS boundary (a write branch's shell is
        // then best effort): the direct write reached the live copy, and the
        // landing refused to write over a copy changed after seeding.
        assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
        assert!(
            (result.residual_gaps.iter()).any(|gap| gap.id.starts_with("project_inputs_refused_")),
            "{result:#?}"
        );
        return;
    }
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    let branch = f.branch_result("impl", "impl-0");
    assert!(
        branch.residual_gaps.iter().any(|gap| {
            gap.id.starts_with("declared_artifact_not_deliverable_")
                && gap.severity.as_deref() == Some("review")
                && gap.description.contains("config/app.toml")
        }),
        "{branch:#?}"
    );
    assert!(!project.join("config/app.toml").exists());

    // EPERM from `sandbox-exec`, EACCES from Landlock.
    let shell = f.shell.lock().unwrap().clone();
    assert!(
        shell[0].contains("Operation not permitted") || shell[0].contains("Permission denied"),
        "the live project root is sealed: {shell:?}"
    );
    // Landed from the branch's copy, through the audited landing.
    assert_eq!(std::fs::read_to_string(&live).unwrap(), "new report\n");
    let log = landings(&f);
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0]["outcome"], json!("applied"));
    assert_eq!(log[0]["path"], json!(REPORT));
    assert_eq!(log[0]["task_ids"], json!(["TASK-001"]));
    let kept = f
        .store
        .run_dir(&f.run)
        .join("write-coordination/project-inputs-replaced/impl/impl-0")
        .join(REPORT);
    assert_eq!(std::fs::read_to_string(kept).unwrap(), "old report\n");
    // Never through the patch.
    assert_eq!(git(&f.repo, &["ls-files", "docs"]), "");
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    // The boundary re-opens nothing in the live project; the agent is told
    // where its copy is.
    let boundary = f.input_stamp("impl", agent_dispatch_port::WRITE_BOUNDARY_INPUT_KEY);
    let writable = boundary["writable"].as_array().unwrap();
    assert!(
        writable.iter().all(|entry| {
            let path = Path::new(entry.as_str().unwrap());
            !path.starts_with(&project) || path.starts_with(project.join(".archon/workflows"))
        }),
        "{writable:?}"
    );
    assert!(
        prompts[0].contains("this branch cannot write it") && prompts[0].contains("lands at"),
        "{}",
        prompts[0]
    );
    assert!(
        prompts[0].contains("config/app.toml") && prompts[0].contains("NOT deliverable"),
        "{}",
        prompts[0]
    );
}

/// A declared artifact whose path in the worktree is the repository's (not
/// ignored there): the branch writes its staging copy, the host lands it,
/// and nothing reaches the repository.
#[tokio::test]
async fn a_declared_artifact_the_repository_would_carry_is_staged_and_landed() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp.path().join("scratch"));
    // Written through the guard an agent's file tool runs under (run-store
    // scope included): it must admit the branch's own staging copy.
    let copy = format!("{COPY}{SUMMARY}");
    let edits = Edits {
        files: vec![
            ("owned.txt", "implemented\n"),
            // Through the file tool's guard, as an agent's `Write` would.
            (
                Box::leak(copy.into_boxed_str()),
                "\u{0}write:{\"cells\":3}\n",
            ),
        ],
        report: vec!["owned.txt"],
        via_adapter: false,
    };
    let (result, _) = wave(&f, &[SUMMARY], edits).await;
    assert!(
        f.shell.lock().unwrap().is_empty(),
        "{:?}",
        f.shell.lock().unwrap()
    );
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    let project = project_root(&f);
    assert_eq!(
        std::fs::read_to_string(project.join(SUMMARY)).unwrap(),
        "{\"cells\":3}\n"
    );
    let log = landings(&f);
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0]["path"], json!(SUMMARY));
    assert_eq!(log[0]["before"], json!("absent"));
    assert_eq!(git(&f.repo, &["ls-files", "reports"]), "");
    assert!(!f.repo.join(SUMMARY).exists());
}

/// Where the OS boundary cannot refuse the direct write, the landing still
/// never writes over a project copy that changed after the branch was
/// seeded: the direct write is not taken for the branch's delivery, and the
/// refusal is logged and reported as a HIGH gap.
#[tokio::test]
async fn a_live_copy_changed_behind_the_landing_is_refused_not_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp.path().join("scratch"));
    let project = project_root(&f);
    let live = project.join(REPORT);
    let direct = format!(
        "\u{0}run:printf 'hacked\\n' > {}",
        support::shell_path(&live)
    );
    let edits = Edits {
        files: vec![
            ("owned.txt", "implemented\n"),
            ("docs/reports/audit.md", "new report\n"),
            ("", Box::leak(direct.into_boxed_str())),
        ],
        report: vec!["owned.txt"],
        via_adapter: false,
    };
    let (result, _) = wave(&f, &[REPORT], edits).await;
    assert_eq!(std::fs::read_to_string(&live).unwrap(), "hacked\n");
    let log = landings(&f);
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0]["outcome"], json!("refused"));
    assert!(
        log[0]["reason"]
            .as_str()
            .unwrap()
            .contains("stale baseline"),
        "{log:?}"
    );
    assert_ne!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    let text = serde_json::to_string(&result).unwrap();
    assert!(text.contains("project_inputs_refused_impl-0"), "{text}");
}
