//! Batch E through the production write-wave seam: a write branch's worktree
//! holds a copy of the acceptance policy's project inputs, a data command the
//! branch runs there changes them, and the landing applies the change to the
//! project root -- where acceptance scratch then reads it -- never through the
//! git patch. Two branches of one wave that both change a shared input: the
//! first lands, the second is refused as stale and says so as a HIGH gap.
//!
//! Before Batch E the worktree held no project data (the command found no
//! registry to extend) and nothing the branch wrote there ever reached the
//! project root, so an acceptance check reading it could never pass.
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::path::{Path, PathBuf};

use archon_workflow::acceptance_scratch::{ScratchPolicy, ScratchRoots};
use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

const REGISTRY: &str = ".archon/lab/data/registry.json";
const INPUT: &str = ".archon/lab/data";

/// A data command: extend the registry the checks read, in place.
fn register(entry: &'static str, target: &'static str) -> Edits {
    Edits {
        files: vec![(target, "implemented\n"), ("", entry)],
        report: vec![target],
        via_adapter: false,
    }
}

const ENTRY_A: &str = "\u{0}run:printf 'entry-a\\n' >> .archon/lab/data/registry.json";
const ENTRY_B: &str = "\u{0}run:printf 'entry-b\\n' >> .archon/lab/data/registry.json";

fn project_root(f: &Fixture) -> PathBuf {
    PathBuf::from(
        project_artifact_context_from_v2_root(f.v2.root())
            .project_root
            .expect("the run has a project root"),
    )
}

fn policy(f: &Fixture, scratch: &Path) -> ScratchPolicy {
    let project = project_root(f).canonicalize().unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    ScratchPolicy {
        repository: f.repo.canonicalize().unwrap(),
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: scratch.to_path_buf(),
        project_inputs: vec![PathBuf::from(INPUT)],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: "/usr/bin:/bin".into(),
        environment: Default::default(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 60,
        output_bytes: 4096,
        scratch_bytes: 1 << 30,
    }
}

/// The fixture repository ignoring project state as the live target does,
/// the project's registry, and the run's launch record of the acceptance
/// policy that names it an input.
fn fixture(scratch: &Path) -> Fixture {
    let f = Fixture::new();
    std::fs::write(f.repo.join(".gitignore"), ".archon/*\n").unwrap();
    git(&f.repo, &["add", ".gitignore"]);
    git(&f.repo, &["commit", "-qm", "ignore project state"]);
    let registry = project_root(&f).join(REGISTRY);
    std::fs::create_dir_all(registry.parent().unwrap()).unwrap();
    std::fs::write(&registry, "seed\n").unwrap();
    let metadata = json!({"observer_snapshot": {"native_execution": {
        "policy": policy(&f, scratch), "source_commit": git(&f.repo, &["rev-parse", "HEAD"])}}});
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
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn a_data_command_in_the_worktree_lands_in_the_project_root_and_acceptance_reads_it() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp.path().join("scratch"));
    let (result, prompts) = f
        .wave_audited(
            "impl",
            vec![(vec!["owned.txt"], register(ENTRY_A, "owned.txt"))],
            None,
        )
        .await;
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");

    // The command extended the SEEDED copy, and the landing applied it.
    let registry = project_root(&f).join(REGISTRY);
    assert_eq!(
        std::fs::read_to_string(&registry).unwrap(),
        "seed\nentry-a\n"
    );
    let log = landings(&f);
    assert_eq!(log.len(), 1, "{log:?}");
    assert_eq!(log[0]["outcome"], json!("applied"));
    assert_eq!(log[0]["path"], json!(REGISTRY));
    assert_eq!(log[0]["task_ids"], json!(["TASK-001"]));
    // Never through the patch: the repository carries the code, not the data.
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    assert_eq!(git(&f.repo, &["ls-files", ".archon"]), "");
    assert!(!f.repo.join(REGISTRY).exists());
    // The agent was told, and its guard may write the copy.
    assert!(prompts[0].contains("Project data:"), "{}", prompts[0]);
    let writable =
        f.input_stamp("impl", agent_dispatch_port::WRITE_BOUNDARY_INPUT_KEY)["writable"].clone();
    assert!(
        writable
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry.as_str().unwrap().ends_with(INPUT)),
        "{writable}"
    );

    // Acceptance scratch overlays the project's inputs on the repository:
    // it reads exactly what the landing applied.
    let head = git(&f.repo, &["rev-parse", "HEAD"]);
    let mut roots = ScratchRoots::prepare(&policy(&f, &temp.path().join("scratch")), &head)
        .expect("scratch prepares");
    assert_eq!(
        std::fs::read_to_string(roots.project().join(REGISTRY)).unwrap(),
        "seed\nentry-a\n"
    );
    assert_eq!(
        std::fs::read_to_string(roots.project().join("owned.txt")).unwrap(),
        "implemented\n"
    );
    roots.cleanup().unwrap();
}

#[tokio::test]
async fn the_second_branch_to_change_a_shared_input_is_refused_stale_as_a_high_gap() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp.path().join("scratch"));
    let result = f
        .wave(
            "impl",
            vec![
                (vec!["owned.txt"], register(ENTRY_A, "owned.txt")),
                (vec!["other.txt"], register(ENTRY_B, "other.txt")),
            ],
        )
        .await;
    // Both were seeded from the same registry; the first to land wins and
    // the second's change is never written over it.
    let registry = project_root(&f).join(REGISTRY);
    assert_eq!(
        std::fs::read_to_string(&registry).unwrap(),
        "seed\nentry-a\n"
    );
    let log = landings(&f);
    let outcomes: Vec<(&str, &str)> = log
        .iter()
        .map(|l| {
            (
                l["item_id"].as_str().unwrap(),
                l["outcome"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(outcomes, [("impl-0", "applied"), ("impl-1", "refused")]);
    assert!(
        log[1]["reason"]
            .as_str()
            .unwrap()
            .contains("stale baseline"),
        "{log:?}"
    );
    // Both patches landed; the refused branch says what did not, as HIGH.
    assert_eq!(git(&f.repo, &["show", "HEAD:other.txt"]), "implemented");
    assert_ne!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    let text = serde_json::to_string(&result).unwrap();
    assert!(text.contains("project_inputs_refused_impl-1"), "{text}");
    assert!(
        text.contains("were NOT applied to the project root"),
        "{text}"
    );
}
