//! Batch K (I1) through the production write-wave seam: a branch that lands
//! a repository TEST FIXTURE as project data -- a byte-identical copy of a
//! tracked `tests/fixtures/` file, or a file naming one as its source -- is
//! refused at landing. Nothing reaches the project root, the refused bytes
//! are kept under the run as evidence, and the branch carries a HIGH finding
//! "repository test fixture landed as project data: <path> from <fixture>".
//!
//! Before Batch K both landed: live, a remediation ingested an 8-bar fixture
//! CSV into the project's dataset registry and its acceptance check passed on
//! it. Both tests fail on 25ff60622 (the landing is applied).
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::path::{Path, PathBuf};

use archon_workflow::acceptance_scratch::ScratchPolicy;
use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

const INPUT: &str = ".archon/lab/data";
const REGISTRY: &str = ".archon/lab/data/registry.json";
const FIXTURE: &str = "crates/lab/tests/fixtures/daily.csv";
const BARS: &str =
    "date,open,high,low,close,volume\n2026-01-02,1,2,0.5,1.5,100\n2026-01-03,1.5,2.5,1,2,120\n";

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
        toolchain_path: std::env::join_paths([archon_shell::resolve_posix_shell()
            .parent()
            .unwrap()])
        .unwrap()
        .into_string()
        .unwrap(),
        environment: Default::default(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 60,
        output_bytes: 4096,
        scratch_bytes: 1 << 30,
        build_cache: None,
    }
}

/// The repository ignores project state and tracks a test fixture; the
/// project holds a registry the acceptance policy names as an input.
fn fixture(scratch: &Path) -> Fixture {
    let f = Fixture::new();
    std::fs::write(f.repo.join(".gitignore"), ".archon/*\n").unwrap();
    std::fs::create_dir_all(f.repo.join(FIXTURE).parent().unwrap()).unwrap();
    std::fs::write(f.repo.join(FIXTURE), BARS).unwrap();
    git(&f.repo, &["add", "."]);
    git(
        &f.repo,
        &["commit", "-qm", "ignore project state; a test fixture"],
    );
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
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|line| line["outcome"] != "intent")
        .collect()
}

/// A data "ingest" that copies the fixture in, and registers it.
const COPY_FIXTURE: &str = "\u{0}run:mkdir -p .archon/lab/data/datasets/spy/raw && cp crates/lab/tests/fixtures/daily.csv .archon/lab/data/datasets/spy/raw/response.csv && printf 'spy\\n' >> .archon/lab/data/registry.json";
/// A request record naming the fixture as the dataset's source.
const NAME_FIXTURE: &str = "\u{0}run:mkdir -p .archon/lab/data/datasets/spy/raw && printf '{\"fixture\": \"crates/lab/tests/fixtures/daily.csv\", \"provider\": \"manual\"}' > .archon/lab/data/datasets/spy/raw/request.json";

fn ingest(command: &'static str) -> Edits {
    Edits {
        files: vec![("owned.txt", "implemented\n"), ("", command)],
        report: vec!["owned.txt"],
        via_adapter: false,
    }
}

fn assert_refused_as_fixture(f: &Fixture, result: &WorkflowV2Result, landed: &str) {
    let project = project_root(f);
    // Nothing of the landing reached the project root.
    assert_eq!(
        std::fs::read_to_string(project.join(REGISTRY)).unwrap(),
        "seed\n"
    );
    assert!(!project.join(landed).exists(), "{landed} landed");
    let log = landings(f);
    assert!(!log.is_empty(), "no decision logged");
    assert!(
        log.iter().all(|line| line["outcome"] == "refused"),
        "{log:?}"
    );
    let reason = log[0]["reason"].as_str().unwrap();
    let expected =
        format!("repository test fixture landed as project data: {landed} from {FIXTURE}");
    assert!(reason.contains(&expected), "{reason}");
    // The refused bytes are kept as evidence.
    let kept = f
        .store
        .run_dir(&f.run)
        .join("write-coordination/project-inputs-refused/impl/impl-0")
        .join(landed);
    assert!(kept.is_file(), "{} not kept", kept.display());
    // A HIGH finding on the unit, in those words; the patch still landed.
    assert_ne!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    let finding = result
        .residual_gaps
        .iter()
        .find(|gap| gap.id.starts_with("project_inputs_test_fixture_impl-0"))
        .unwrap_or_else(|| panic!("no fixture finding: {result:#?}"));
    assert_eq!(finding.severity.as_deref(), Some("high"));
    assert!(
        finding.description.starts_with(&expected),
        "{}",
        finding.description
    );
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
}

#[tokio::test]
async fn a_byte_identical_copy_of_a_tracked_test_fixture_is_refused_as_a_high_finding() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp.path().join("scratch"));
    let result = f
        .wave("impl", vec![(vec!["owned.txt"], ingest(COPY_FIXTURE))])
        .await;
    assert_refused_as_fixture(
        &f,
        &result,
        ".archon/lab/data/datasets/spy/raw/response.csv",
    );
}

#[tokio::test]
async fn a_landed_record_naming_a_tracked_test_fixture_as_its_source_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let f = fixture(&temp.path().join("scratch"));
    let result = f
        .wave("impl", vec![(vec!["owned.txt"], ingest(NAME_FIXTURE))])
        .await;
    assert_refused_as_fixture(
        &f,
        &result,
        ".archon/lab/data/datasets/spy/raw/request.json",
    );
}
