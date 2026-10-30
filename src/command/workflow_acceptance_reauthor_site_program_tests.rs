//! F6 through the re-author loop at a real scratch site: a check runs a
//! tool's subcommand that the site's search path cannot resolve. When the
//! host has the program the site lacks, the site the host built is at
//! fault: the check stays the host's (pending), never its author's. Only
//! when neither the site nor the host has it does the author get a finding
//! and replace the check. Issue 548: no host repair makes the site provide
//! it, so both cases now go to the author in the same budget.

use super::*;
use crate::command::workflow_task_set::executability::HOST_PATH;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use crate::command::workflow_task_set::republish::test_fixture::{NO_SEEDS, criterion};
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, GapPolicy, PrdIdentity, TrustedCwd, content_digest,
};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const ID: &str = "SUP-X-001";
/// A tool that dispatches `extra` to a program `fixturetool-extra`, which
/// its search path does not have, and lists its built-in commands.
const TOOL: &str = "#!/bin/sh\ncase \"$1\" in\n  --list) printf 'Installed commands:\\n    build\\n    run\\n' ;;\n  extra) echo 'error: no such command: `extra`' >&2; exit 2 ;;\n  *) exit 0 ;;\nesac\n";
const RUNS_EXTRA: &str = "fixturetool extra --check";
const OWN_ASSERTION: &str = "grep -q 'fixture text' src/state.txt";

fn git(repo: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {}", args.join(" "));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    temp: tempfile::TempDir,
    project: PathBuf,
    tasks: PathBuf,
    prd: PathBuf,
    site_bin: PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let repository = temp.path().join("repo");
    let site_bin = temp.path().join("site-bin");
    let tasks = project.join("tasks/PRD-X");
    let prd = project.join("prds/PRD-X.md");
    for dir in [
        &repository.join("src"),
        &site_bin,
        &tasks,
        &prd.parent().unwrap().to_path_buf(),
    ] {
        std::fs::create_dir_all(dir).unwrap();
    }
    git(&repository, &["init", "-q"]);
    git(
        &repository,
        &["config", "user.email", "test@example.invalid"],
    );
    git(&repository, &["config", "user.name", "test"]);
    std::fs::write(repository.join("src/state.txt"), "fixture text\n").unwrap();
    git(&repository, &["add", "."]);
    git(&repository, &["commit", "-qm", "baseline"]);
    let commit = git(&repository, &["rev-parse", "HEAD"]);
    let prd_text = "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | The state file exists. |\n| SUP-X-001 | The state file keeps its text. |\n";
    std::fs::write(&prd, prd_text).unwrap();
    let in_repo = |mut entry: archon_workflow::task_set_contract::AcceptanceCriterion| {
        if let AcceptanceCheck::Command { cwd, .. } = &mut entry.check {
            *cwd = TrustedCwd::RepoRoot;
        }
        entry
    };
    let contract = AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "prds/PRD-X.md".into(),
            digest: content_digest(prd_text.as_bytes()),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: Default::default(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance: vec![in_repo(criterion(
            "AC-X-001",
            "test -f src/state.txt",
            true,
        ))],
        supplementary: vec![in_repo(criterion(ID, "test -f src/state.txt", false))],
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
        decomposition_run_id: "site-program-fixture".into(),
        recorded_at: "2026-09-01T00:00:00Z".into(),
    };
    archon_workflow::repository_record::write_repository_record(&tasks, &record).unwrap();
    executable(&site_bin.join("fixturetool"), TOOL);
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    std::fs::write(
        project.join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path={:?}\ntimeout_secs=300\noutput_bytes=8192\nscratch_bytes=16777216\n",
            repository.display().to_string(),
            temp.path().join("scratch").display().to_string(),
            format!("{}:/usr/bin:/bin", site_bin.display()),
        ),
    )
    .unwrap();
    Fixture {
        temp,
        project,
        tasks,
        prd,
        site_bin,
    }
}

/// Re-author `ID` at `fixture`'s scratch site, the host's search path
/// `host_path`: the first reply runs `fixturetool extra`, later ones assert
/// with the site's own programs.
async fn reauthor(
    fixture: &Fixture,
    host_path: String,
) -> (Result<ReauthorResult, anyhow::Error>, Vec<String>) {
    HOST_PATH.with(|path| *path.borrow_mut() = Some(host_path));
    let probe = HostProbe::for_task_set(&fixture.project, &fixture.tasks);
    let client = ScriptedAuthorJudge::new(
        |entry, attempt| match attempt {
            1 => command_entry(entry, RUNS_EXTRA),
            _ => command_entry(entry, OWN_ASSERTION),
        },
        |_, _| true,
    );
    let named = BTreeSet::from([ID.to_string()]);
    let result = reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: &fixture.project,
            tasks_root: &fixture.tasks,
            prd_path: &fixture.prd,
            ids: &named,
            gate: ReauthorGate {
                probe: &probe,
                seeds: &NO_SEEDS,
            },
            trigger: "site program",
        },
        &AuthorScope::for_task_set_without_cache(&fixture.project, &fixture.tasks, &fixture.prd)
            .unwrap(),
    )
    .await;
    HOST_PATH.with(|path| *path.borrow_mut() = None);
    let prompts = client.prompts.lock().unwrap().clone();
    (result, prompts)
}

/// Issue 548 (replaces `a_subcommand_the_host_has_but_the_site_lacks_stays_pending_for_the_host`):
/// no host repair makes the site provide the program the host has, so the
/// check goes back to its author in the same re-author budget, with a true
/// text, and its replacement publishes: nothing stays pending for a repair
/// that does not exist.
#[tokio::test]
async fn a_subcommand_the_host_has_but_the_site_lacks_goes_to_its_author_and_publishes() {
    let fixture = fixture();
    let host_bin = fixture.temp.path().join("host-bin");
    std::fs::create_dir_all(&host_bin).unwrap();
    executable(&host_bin.join("fixturetool-extra"), "#!/bin/sh\nexit 0\n");
    let host_path = format!(
        "{}:{}:/usr/bin:/bin",
        host_bin.display(),
        fixture.site_bin.display()
    );
    let (result, prompts) = reauthor(&fixture, host_path).await;
    let result = result.expect("the author's replacement publishes");
    assert_eq!(
        prompts.len(),
        2,
        "the author is asked again in the same budget: {prompts:#?}"
    );
    let finding = &prompts[1];
    assert!(finding.contains("though the host has one at"), "{finding}");
    assert!(
        finding.contains("check the criterion with what the site provides"),
        "{finding}"
    );
    assert!(!finding.contains("never its author's"), "{finding}");
    assert!(!finding.contains("host's to repair"), "{finding}");
    assert!(result.unproven.is_empty(), "{:?}", result.unproven);
    assert!(result.published);
}

#[tokio::test]
async fn a_subcommand_neither_site_nor_host_has_goes_to_its_author() {
    let fixture = fixture();
    let host_path = format!("{}:/usr/bin:/bin", fixture.site_bin.display());
    let (result, prompts) = reauthor(&fixture, host_path).await;
    let result = result.expect("the author's replacement publishes");
    assert_eq!(prompts.len(), 2, "{prompts:#?}");
    assert!(
        prompts[1].contains("program unavailable at the probe site")
            && prompts[1].contains("nor on the host's"),
        "{}",
        prompts[1]
    );
    assert!(result.unproven.is_empty(), "{:?}", result.unproven);
    assert!(result.published);
}
