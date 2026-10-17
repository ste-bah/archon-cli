//! One temporary project per cell: the fixed fixture, a git repository, a
//! local fake provider, and the CLI children run against them.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::deadline::{isolate_child, run};
use crate::server;
use crate::snapshot::{result_store_records, strip_timestamps};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/offline_decomposition"
);

pub struct TestProject {
    temp: Option<tempfile::TempDir>,
    root: PathBuf,
    pub provider: server::FakeProvider,
}

impl TestProject {
    pub fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        let root = self.path().to_path_buf();
        let name = root.file_name().unwrap().to_string_lossy();
        let harness = root.parent().unwrap().join(format!("{name}-harness"));
        if std::thread::panicking() {
            crate::diagnostics::print_failure(&root, &self.provider.requests);
            if let Some(temp) = self.temp.take() {
                let _ = temp.keep();
            }
        }
        if !std::thread::panicking() {
            let _ = fs::remove_dir_all(harness);
        }
    }
}

pub fn project() -> TestProject {
    project_with(server::Plan {
        variant: server::Variant::Golden,
        block: None,
    })
}

pub fn project_with_provider(blocked_role: Option<&str>) -> TestProject {
    project_with(server::Plan {
        variant: server::Variant::Golden,
        block: blocked_role.map(|role| (role.to_string(), 1)),
    })
}

pub fn project_with(plan: server::Plan) -> TestProject {
    project_with_files(plan, &[])
}

/// The fixture with `files` (project-relative path, contents) written over
/// it before the repository's fixture commit.
pub fn project_with_files(plan: server::Plan, files: &[(&str, String)]) -> TestProject {
    let temp = tempfile::tempdir().unwrap();
    let root = if plan.variant == server::Variant::SiblingCheck {
        temp.path().join("project")
    } else {
        temp.path().to_path_buf()
    };
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(root.join("repository")).unwrap();
    fs::create_dir_all(root.join("tasks")).unwrap();
    copy_tree(
        &PathBuf::from(FIXTURE).join("repository"),
        &root.join("repository"),
    );
    copy_tree(
        &PathBuf::from(FIXTURE).join(".archon"),
        &root.join(".archon"),
    );
    fs::copy(PathBuf::from(FIXTURE).join("prd.md"), root.join("prd.md")).unwrap();
    if plan.variant == server::Variant::SiblingCheck {
        let sibling = root.parent().unwrap().join("shared-check");
        fs::create_dir_all(&sibling).unwrap();
        fs::write(sibling.join("marker.txt"), "fixture marker\n").unwrap();
        git_in_directory(&sibling, &["init", "-q"]);
        git_in_directory(&sibling, &["add", "."]);
        git_in_directory(
            &sibling,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "sibling fixture",
            ],
        );
    }
    for (path, contents) in files {
        fs::write(root.join(path), contents).unwrap();
    }
    git(&root, &["init", "-q"]);
    git(&root, &["add", "."]);
    git(
        &root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ],
    );
    let provider = server::FakeProvider::start_with(plan);
    let config = format!(
        r#"
[api]
default_model = "claude-sonnet-4-6"
default_effort = "low"
thinking_budget = 0
base_url = "{}"
auth = "required"
[llm]
provider = "anthropic"
[llm.openai]
base_url = "{}"
[llm.local]
base_url = "{}"
[workflow]
gate_mode = "observe"
repository_root = "repository"
[subagent]
max_concurrent = 1
"#,
        provider.base_url,
        provider.base_url.trim_end_matches("/v1/messages"),
        provider.base_url.trim_end_matches("/v1/messages")
    );
    fs::write(root.join("config.toml"), &config).unwrap();
    fs::write(root.join(".archon/config.toml"), &config).unwrap();
    TestProject {
        temp: Some(temp),
        root,
        provider,
    }
}

fn git(root: &Path, args: &[&str]) {
    git_in_directory(&root.join("repository"), args);
}

fn git_in_directory(directory: &Path, args: &[&str]) {
    // The fixture commit must not depend on the runner's git settings: a
    // Windows runner's core.autocrlf would otherwise rewrite line ends.
    let status = Command::new("git")
        .current_dir(directory)
        .args(["-c", "core.autocrlf=false"])
        .args(args)
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}: {status}");
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let destination = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

pub fn decompose_args() -> [&'static str; 9] {
    [
        "workflow",
        "decompose",
        "--prd",
        "prd.md",
        "--tasks",
        "tasks/PRD-X",
        "--repository",
        "repository",
        "--yes",
    ]
}

/// The decomposition child, with the given test-seam environment
/// (`src/command/workflow_test_seams.rs`) on top of the isolated one.
pub fn spawn_decompose(root: &Path, seams: &[(&str, String)]) -> std::process::Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_archon"));
    let harness = crate::deadline::harness_root(root);
    isolate_child(&mut command, root, &harness);
    command
        .current_dir(root)
        .args(decompose_args())
        .envs(seams.iter().map(|(name, value)| (*name, value.as_str())))
        .stdout(fs::File::create(harness.join("decompose.stdout")).unwrap())
        .stderr(fs::File::create(harness.join("decompose.stderr")).unwrap())
        .spawn()
        .unwrap()
}

/// Accepted, non-reused author attempts per call id, from the run's events.
pub fn accepted_call_counts(root: &Path) -> BTreeMap<String, usize> {
    let events = fs::read_to_string(one_run(root).join("events.jsonl")).unwrap();
    let mut counts = BTreeMap::new();
    for line in events.lines() {
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        if event["kind"] == "author_attempt_completed"
            && event["detail"]["status"] == "accepted"
            && event["detail"]["reused"] != true
            && let Some(call_id) = event["detail"]["call_id"].as_str()
        {
            *counts.entry(call_id.to_string()).or_default() += 1;
        }
    }
    counts
}

/// Every author call accepted before the resume keeps its count; the
/// interrupted author call (by id prefix), if any, is accepted exactly once
/// after it, and was not accepted before it.
pub fn assert_only_call_accepted_again(
    before: &BTreeMap<String, usize>,
    after: &BTreeMap<String, usize>,
    interrupted: Option<&str>,
) {
    for (call_id, prior) in before {
        assert_eq!(
            after.get(call_id),
            Some(prior),
            "unexpected accepted call replay for {call_id}: before={before:?} after={after:?}"
        );
    }
    if let Some(prefix) = interrupted {
        let accepted_after = after
            .iter()
            .filter(|(id, _)| id.starts_with(prefix) && !before.contains_key(*id))
            .map(|(_, count)| *count)
            .collect::<Vec<_>>();
        assert_eq!(
            accepted_after,
            [1],
            "the interrupted call {prefix} must be accepted once, after resume: before={before:?} after={after:?}"
        );
    }
}

pub fn resume(root: &Path, run_id: &str) -> Output {
    run(root, &["workflow", "resume", "--live", "--yes", run_id])
}

pub fn one_run(root: &Path) -> PathBuf {
    let runs = fs::read_dir(root.join(".archon/workflows")).unwrap();
    let mut matches = runs
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("state.json").is_file());
    let run = matches.next().expect("decomposition result-store run");
    assert!(matches.next().is_none(), "fixture should create one run");
    run
}

pub fn run_id(root: &Path) -> String {
    one_run(root)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

pub fn assert_no_unknown_prompts(provider: &server::FakeProvider) {
    let unknown = provider
        .requests
        .snapshot()
        .into_iter()
        .filter(|request| request.role == "unknown")
        .collect::<Vec<_>>();
    assert!(
        unknown.is_empty(),
        "unknown fake-provider prompts: {unknown:?}"
    );
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FinalSnapshot {
    pub acceptance: serde_json::Value,
    pub skeleton: serde_json::Value,
    pub task_bodies: Vec<(String, String)>,
    pub records: Vec<(String, serde_json::Value)>,
}

/// The final artifacts and normalised result-store records. `interrupted`
/// names the calls a fault interrupted (`snapshot::lifecycle`).
pub fn final_snapshot(project: &TestProject, interrupted: &[&str]) -> FinalSnapshot {
    let root = project.path();
    let run = one_run(root);
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(run.join("state.json")).unwrap()).unwrap();
    assert_eq!(state["status"], "completed", "final run state: {state}");
    let mut acceptance: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("tasks/PRD-X/acceptance-contract.json")).unwrap(),
    )
    .unwrap();
    strip_timestamps(&mut acceptance);
    if let Some(prd) = acceptance
        .get_mut("prd")
        .and_then(serde_json::Value::as_object_mut)
    {
        prd.remove("path");
    }
    let mut skeleton: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("tasks/PRD-X/task-skeleton.json")).unwrap())
            .unwrap();
    strip_timestamps(&mut skeleton);
    let task_bodies = ["TASK-X-010.md", "TASK-X-020.md"]
        .into_iter()
        .map(|name| {
            (
                name.into(),
                fs::read_to_string(root.join("tasks/PRD-X").join(name)).unwrap(),
            )
        })
        .collect();
    let records = result_store_records(
        &run,
        root,
        &project.provider.base_url,
        run.file_name().unwrap().to_str().unwrap(),
        interrupted,
    );
    assert!(
        !records.is_empty(),
        "the result store must retain host-call records"
    );
    FinalSnapshot {
        acceptance,
        skeleton,
        task_bodies,
        records,
    }
}

fn golden_requests_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/offline_decomposition_golden_requests.json")
}

/// The fresh run's provider requests per call equal the fixed golden counts
/// (rewritten with the golden snapshot under `ARCHON_UPDATE_GOLDEN=1`).
pub fn assert_golden_requests(actual: &BTreeMap<String, usize>) {
    if std::env::var_os("ARCHON_UPDATE_GOLDEN").as_deref() == Some(std::ffi::OsStr::new("1")) {
        let mut bytes = serde_json::to_vec_pretty(actual).unwrap();
        bytes.push(b'\n');
        fs::write(golden_requests_path(), bytes).unwrap();
        return;
    }
    assert_eq!(
        actual,
        &read_golden_requests(),
        "fresh run's provider requests per call differ from the fixed golden counts"
    );
}

pub fn read_golden_requests() -> BTreeMap<String, usize> {
    let path = golden_requests_path();
    serde_json::from_slice(&fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "reading golden request counts {}: {error}; generate them with ARCHON_UPDATE_GOLDEN=1",
            path.display()
        )
    }))
    .unwrap()
}

/// `actual == expected`, else a panic naming every differing JSON pointer
/// (at most 40) with both values, instead of two unreadable dumps.
pub fn assert_same_json(label: &str, actual: &serde_json::Value, expected: &serde_json::Value) {
    fn walk(a: &serde_json::Value, e: &serde_json::Value, at: String, out: &mut Vec<String>) {
        match (a, e) {
            (serde_json::Value::Object(a), serde_json::Value::Object(e)) => {
                let keys = a
                    .keys()
                    .chain(e.keys())
                    .collect::<std::collections::BTreeSet<_>>();
                for key in keys {
                    match (a.get(key), e.get(key)) {
                        (Some(a), Some(e)) => walk(a, e, format!("{at}/{key}"), out),
                        (Some(_), None) => out.push(format!(
                            "missing on one side: {at}/{key} (absent in expected)"
                        )),
                        (None, Some(_)) => out.push(format!(
                            "missing on one side: {at}/{key} (absent in actual)"
                        )),
                        (None, None) => unreachable!("the key came from one of the maps"),
                    }
                }
            }
            (serde_json::Value::Array(a), serde_json::Value::Array(e)) if a.len() == e.len() => {
                for (index, (a, e)) in a.iter().zip(e).enumerate() {
                    walk(a, e, format!("{at}/{index}"), out);
                }
            }
            _ if a != e => {
                let short = |value: &serde_json::Value| {
                    value.to_string().chars().take(300).collect::<String>()
                };
                out.push(format!("{at}: actual {} expected {}", short(a), short(e)));
            }
            _ => {}
        }
    }
    let mut differences = Vec::new();
    walk(actual, expected, String::new(), &mut differences);
    assert!(
        differences.is_empty(),
        "{label}: {} differences:\n{}",
        differences.len(),
        differences
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The snapshot with its records keyed by call (method, author call id or
/// host `commandId`, then order), so a difference names the call it is in.
pub fn keyed(mut snapshot: serde_json::Value) -> serde_json::Value {
    let records = snapshot["records"].as_array().cloned().unwrap_or_default();
    let mut keyed = serde_json::Map::new();
    for record in records {
        let record = &record[1];
        let call = &record["call"];
        let method = call["method"].as_str().unwrap_or_default();
        let name = call
            .pointer("/options/extra/commandId")
            .and_then(serde_json::Value::as_str)
            .or_else(|| call["id"].as_str())
            .unwrap_or_default();
        let mut n = 1;
        while keyed.contains_key(&format!("{method}:{name}:{n}")) {
            n += 1;
        }
        keyed.insert(format!("{method}:{name}:{n}"), record.clone());
    }
    snapshot["records"] = serde_json::Value::Object(keyed);
    snapshot
}

#[test]
fn assert_same_json_tells_a_missing_key_from_a_null_value() {
    let absent = serde_json::json!({"a": 1});
    let null = serde_json::json!({"a": 1, "b": null});
    let result = std::panic::catch_unwind(|| assert_same_json("probe", &absent, &null));
    let message = result
        .expect_err("absent and null must differ")
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert!(message.contains("missing on one side: /b"), "{message}");
    assert_same_json("probe", &null, &null.clone());
}
