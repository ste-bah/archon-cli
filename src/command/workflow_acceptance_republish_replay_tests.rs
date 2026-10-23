//! Replays two recorded author replies against a real repository/project
//! sibling layout. The probe's hermetic copy deliberately renames the repo.

use super::*;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use crate::command::workflow_task_set::republish::test_fixture::NO_SEEDS;
use archon_workflow::task_set_contract::{
    AcceptanceContract, GapPolicy, PrdIdentity, TrustedCwd, content_digest,
};
use std::path::{Path, PathBuf};

const RESUME_19: &str = r#"{"id":"SUP-REQ-F-083","criterion":"A missing `native_interval` field means `false`.","check":{"kind":"command","command":"cargo test --quiet --release --manifest-path ../archon-cli/Cargo.toml -p archon-trading --lib data_lake::tests::missing_flags_deserialize_false_and_missing_quality_deserializes_degraded && cargo test --quiet --release --manifest-path ../archon-cli/Cargo.toml -p archon-trading --lib data_lake::tests::in_memory_v1_read_upgrade_leaves_disk_bytes_identical","cwd":"repo_root"},"gap_permitted":false,"covers":["REQ-F-083"],"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}}"#;

const RESUME_20: &str = r#"{"id":"SUP-REQ-F-083","criterion":"A missing `native_interval` field means `false`.","check":{"kind":"command","command_block":true,"cwd":"repo_root"},"gap_permitted":false,"covers":["REQ-F-083"],"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}}

```check
set -e
test -f crates/archon-trading/src/data_lake/tests.rs
cp crates/archon-trading/src/data_lake/tests.rs /tmp/sup_req_dl_083_tests.rs.bak
trap 'mv /tmp/sup_req_dl_083_tests.rs.bak crates/archon-trading/src/data_lake/tests.rs' EXIT
mv crates/archon-trading/src/data_lake/tests.rs /tmp/sup_req_dl_083_absent.rs
if cargo nextest run -p archon-trading --test-threads=1 2>/dev/null; then
  echo "REGRESSION GUARD FAILED: criterion holds with tests.rs moved aside"
  exit 1
fi
mv /tmp/sup_req_dl_083_absent.rs crates/archon-trading/src/data_lake/tests.rs
cargo nextest run -p archon-trading missing_flags_deserialize_false_and_missing_quality_deserializes_degraded req_dl_020_gate_rejects_via_new_constructor_with_dataset_id --test-threads=1
```"#;

fn git(repo: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

struct ReplayFixture {
    _temp: tempfile::TempDir,
    project: PathBuf,
    repository: PathBuf,
    tasks: PathBuf,
    prd: PathBuf,
    probe: HostProbe,
}

fn replay_fixture() -> ReplayFixture {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let repository = temp.path().join("archon-cli");
    let tool_bin = temp.path().join("site-bin");
    let scratch = temp.path().join("scratch");
    let tasks = project.join("tasks/PRD-F");
    let prd = project.join("prds/PRD-F.md");
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::create_dir_all(&tool_bin).unwrap();
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    git(&repository, &["init", "-q"]);
    git(
        &repository,
        &["config", "user.email", "test@example.invalid"],
    );
    git(&repository, &["config", "user.name", "test"]);
    std::fs::write(repository.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
    let test_file = repository.join("crates/archon-trading/src/data_lake/tests.rs");
    std::fs::create_dir_all(test_file.parent().unwrap()).unwrap();
    std::fs::write(&test_file, "// replay fixture\n").unwrap();
    git(&repository, &["add", "."]);
    git(&repository, &["commit", "-qm", "baseline"]);
    let commit = git(&repository, &["rev-parse", "HEAD"]);

    let prd_text = "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-F-001 | Repository tests exist. |\n| SUP-REQ-F-083 | A missing native_interval field means false. |\n";
    std::fs::write(&prd, prd_text).unwrap();
    let mut entry = crate::command::workflow_task_set::republish::test_fixture::criterion(
        "SUP-REQ-F-083",
        "test -f Cargo.toml",
        false,
    );
    entry.criterion = "A missing `native_interval` field means `false`.".into();
    entry.covers = vec!["REQ-F-083".into()];
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { cwd, .. } =
        &mut entry.check
    {
        *cwd = TrustedCwd::RepoRoot;
    }
    let mut accepted = crate::command::workflow_task_set::republish::test_fixture::criterion(
        "AC-F-001",
        "test -f Cargo.toml",
        true,
    );
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { cwd, .. } =
        &mut accepted.check
    {
        *cwd = TrustedCwd::RepoRoot;
    }
    let contract = AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "prds/PRD-F.md".into(),
            digest: content_digest(prd_text.as_bytes()),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: Default::default(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance: vec![accepted],
        supplementary: vec![entry],
    };
    crate::command::workflow_task_set::republish::test_fixture::write_chain(
        &project,
        &tasks,
        &contract,
        archon_workflow::task_set_contract::FreezeGateMode::Observe,
        1,
    );
    let record = archon_workflow::repository_record::RepositoryRecordV1 {
        schema_version: archon_workflow::repository_record::REPOSITORY_RECORD_SCHEMA_VERSION,
        repository_root: repository.canonicalize().unwrap().display().to_string(),
        base_commit: commit,
        decomposition_run_id: "replay-fixture".into(),
        recorded_at: "2026-09-01T00:00:00Z".into(),
    };
    archon_workflow::repository_record::write_repository_record(&tasks, &record).unwrap();
    // The shim keeps the replay offline and preserves the two host failure
    // classes: a missing sibling manifest, and cargo's unavailable nextest
    // subcommand at the site, which it lists no built-in command for.
    let cargo = tool_bin.join("cargo");
    std::fs::write(
        &cargo,
        "#!/bin/sh\ncase \"$*\" in\n  --list) printf 'Installed Commands:\\n    build\\n    test\\n' ;;\n  *--manifest-path*)\n    if [ -f ../archon-cli/Cargo.toml ]; then exit 0; fi\n    echo 'error: manifest path `../archon-cli/Cargo.toml` does not exist' >&2\n    exit 101\n    ;;\n  *nextest*) echo 'error: no such command: `nextest`' >&2; exit 101 ;;\n  *) exit 0 ;;\nesac\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config_dir = project.join(".archon");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path={:?}\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
            repository.display().to_string(),
            scratch.display().to_string(),
            format!("{}:/usr/bin:/bin", tool_bin.display()),
        ),
    )
    .unwrap();
    let probe = HostProbe::for_task_set(&project, &tasks);
    ReplayFixture {
        _temp: temp,
        project,
        repository,
        tasks,
        prd,
        probe,
    }
}

#[tokio::test]
async fn recorded_resume_failures_become_author_findings_and_allow_only_the_valid_repair_to_publish()
 {
    let fixture = replay_fixture();
    // The recorded failure is a subcommand neither the site nor the host
    // has (F6): pin the host's search path so the machine running this
    // test cannot supply it.
    let site_path = format!(
        "{}:/usr/bin:/bin",
        fixture.project.parent().unwrap().join("site-bin").display()
    );
    crate::command::workflow_task_set::executability::HOST_PATH
        .with(|path| *path.borrow_mut() = Some(site_path));
    let named = BTreeSet::from(["SUP-REQ-F-083".to_string()]);
    let initial_pin: archon_workflow::task_set_contract::AcceptancePin = serde_json::from_slice(
        &std::fs::read(crate::command::workflow_task_set::acceptance_pin_path(
            &fixture.project,
            &fixture.tasks,
        ))
        .unwrap(),
    )
    .unwrap();
    let client = ScriptedAuthorJudge::new(
        |entry, attempt| match attempt {
            1 => RESUME_19.to_string(),
            2 => RESUME_20.to_string(),
            _ => command_entry(
                entry,
                "grep -q 'replay fixture' crates/archon-trading/src/data_lake/tests.rs",
            ),
        },
        |_, _| true,
    );
    let scope = AuthorScope::for_task_set(&fixture.project, &fixture.tasks, &fixture.prd)
        .expect("repository record identifies the sibling repository");
    let result = reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: &fixture.project,
            tasks_root: &fixture.tasks,
            prd_path: &fixture.prd,
            ids: &named,
            gate: ReauthorGate {
                probe: &fixture.probe,
                seeds: &NO_SEEDS,
            },
            trigger: "recorded resume replay",
        },
        &scope,
    )
    .await
    .expect("the valid third reply publishes after both recorded findings");

    crate::command::workflow_task_set::executability::HOST_PATH
        .with(|path| *path.borrow_mut() = None);
    let prompts = client.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 3, "both drafts were shown back as findings");
    assert!(
        prompts[1].contains("../archon-cli/Cargo.toml")
            && prompts[1].contains("outside the tree it ran in"),
        "{}",
        prompts[1]
    );
    assert!(prompts[2].contains("cargo nextest"), "{}", prompts[2]);
    assert!(
        prompts[2].contains("program unavailable at the probe site")
            && prompts[2].contains("cargo-nextest"),
        "the missing site program is an author finding, not HostUnproven/operational: {}",
        prompts[2]
    );
    for (label, prompt, needle) in [
        ("path", &prompts[1], "outside the tree it ran in"),
        (
            "site-program",
            &prompts[2],
            "program unavailable at the probe site",
        ),
    ] {
        let finding = prompt
            .lines()
            .find(|line| line.contains(needle))
            .expect("the author prompt includes the matching finding");
        eprintln!("REPLAY_FINDING_{label}: {finding}");
    }
    drop(prompts);
    assert!(
        result.unproven.is_empty(),
        "author findings are not HostUnproven: {:?}",
        result.unproven
    );
    assert!(result.published);
    assert_eq!(fixture.repository.file_name().unwrap(), "archon-cli");
    assert_eq!(
        fixture.repository.parent().unwrap(),
        fixture.project.parent().unwrap(),
        "the repository and project are siblings"
    );
    let pin: archon_workflow::task_set_contract::AcceptancePin = serde_json::from_slice(
        &std::fs::read(crate::command::workflow_task_set::acceptance_pin_path(
            &fixture.project,
            &fixture.tasks,
        ))
        .unwrap(),
    )
    .unwrap();
    assert_ne!(pin.acceptance_digest, initial_pin.acceptance_digest);
    let changed_ids = pin.lineage.last().unwrap().reauthored_ids.clone();
    assert_eq!(
        changed_ids, named,
        "only the published id is on the pin transition"
    );
}
