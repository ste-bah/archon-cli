//! Issue 548 through the whole-set freeze at a configured scratch site.
//!
//! A check that runs a program the host has but the site lacks is never
//! repaired by the host (nothing adds a program to the site it built), so
//! it goes to its author in the same budget and the freeze publishes with
//! no resume. A check the host truly cannot prove (an inconclusive probe of
//! its own, retried without a strike) still never holds back another
//! check's authored finding.

use super::*;

/// The fixture of `outside_set(VACUOUS)` with a second check `AC-X-002`
/// that runs `second`, a scratch policy whose toolchain path is
/// `toolchain`, and the host's search path `host_path`.
fn site_set(
    second: &str,
    toolchain: &str,
    host_path: String,
) -> (tempfile::TempDir, tempfile::TempDir, PathBuf, PathBuf) {
    use crate::command::workflow_task_set::executability::HOST_PATH;
    let (project, outside, tasks, prd) = outside_set(VACUOUS);
    std::fs::write(
        &prd,
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | output is valid |\n| AC-X-002 | the tool reports success |\n",
    )
    .unwrap();
    let path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut draft: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut entry = draft["acceptance"][0].clone();
    entry["id"] = "AC-X-002".into();
    entry["check"]["command"] = second.into();
    draft["acceptance"].as_array_mut().unwrap().push(entry);
    std::fs::write(&path, serde_json::to_vec(&draft).unwrap()).unwrap();
    std::fs::create_dir_all(project.path().join(".archon")).unwrap();
    std::fs::write(
        project.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path={:?}\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=16777216\n",
            outside.path().join("repo").display().to_string(),
            outside.path().join("scratch").display().to_string(),
            toolchain,
        ),
    )
    .unwrap();
    HOST_PATH.with(|path| *path.borrow_mut() = Some(host_path));
    (project, outside, tasks, prd)
}

/// An executable shell script at `path`.
fn executable(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

/// The freeze of `project`'s task set, by the author and judge `client`.
async fn freeze(
    (project, tasks, prd): (&Path, &Path, &Path),
    client: Arc<ScriptedAuthorJudge>,
) -> anyhow::Result<PreparedAcceptanceFreeze> {
    let scope = AuthorScope::for_task_set_without_cache(project, tasks, prd)
        .expect("the task set's repository record is believed");
    prepare_acceptance_freeze_reauthoring(project, tasks, prd, GateMode::Enforce, client, &scope)
        .await
}

/// Issue 548, the test the issue asks for: a check whose site lacks a
/// program the host has, which the host cannot make the site provide,
/// reaches its author on the first attempt with a true text, and the freeze
/// publishes its replacement without a resume.
#[tokio::test]
async fn a_site_lacking_check_reaches_its_author_and_the_freeze_publishes_without_a_resume() {
    use crate::command::workflow_task_set::executability::HOST_PATH;
    let temp = tempfile::tempdir().unwrap();
    let host_bin = temp.path().join("host-bin");
    executable(
        &host_bin.join("archon-host-only-tool"),
        "#!/bin/sh\nexit 0\n",
    );
    let (project, _outside, tasks, prd) = site_set(
        "archon-host-only-tool && test -f out.json",
        "/usr/bin:/bin:/usr/sbin:/sbin",
        format!("{}:/usr/bin:/bin", host_bin.display()),
    );
    let client = Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, SOUND),
        |_, _| true,
    ));
    let prepared = freeze((project.path(), &tasks, &prd), client.clone()).await;
    HOST_PATH.with(|path| *path.borrow_mut() = None);
    let prepared = prepared.expect("the replacement publishes in the same attempt, no resume");
    assert!(prepared.findings.is_empty(), "{:?}", prepared.findings);
    let prompts = client.prompts.lock().unwrap().clone();
    let finding = (prompts.iter())
        .find(|prompt| prompt.contains("archon-host-only-tool"))
        .unwrap_or_else(|| panic!("its author was asked: {prompts:#?}"));
    assert!(finding.contains("AC-X-002"), "{finding}");
    assert!(
        finding.contains(
            "the host has `archon-host-only-tool`, but the check's site does not provide it"
        ),
        "{finding}"
    );
    assert!(
        finding.contains("check the criterion with what the site provides"),
        "{finding}"
    );
    for false_claim in [
        "never its author's",
        "host's to repair",
        "repair the scratch site",
    ] {
        assert!(!finding.contains(false_claim), "{false_claim}: {finding}");
    }
    let contract = prepared.contract().unwrap();
    let replaced = (contract.acceptance.iter())
        .find(|entry| entry.id == "AC-X-002")
        .expect("AC-X-002 is published");
    assert!(matches!(
        &replaced.check,
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } if command == SOUND
    ));
}

/// N1 / R5-3 (replaces `an_unproven_check_does_not_hold_back_another_checks_authored_finding`,
/// whose unproven check was a site lacking a program the host has: Issue
/// 548 sends that one to its author). A check the host could not prove --
/// its listing probe of the tool the check runs gave no output, which the
/// host retries on every attempt without a strike or author finding --
/// does not hold back another check's authored finding: that author is
/// asked first (per-target results, as a keep-going build keeps them), the
/// freeze still ends incomplete and resumable for the unproven check alone,
/// and the repair is kept: the next attempt reuses it without asking its
/// author or the judge again.
#[tokio::test]
async fn an_unproven_check_does_not_hold_back_another_checks_authored_finding_issue548() {
    use crate::command::workflow_task_set::executability::HOST_PATH;
    let temp = tempfile::tempdir().unwrap();
    let site_bin = temp.path().join("site-bin");
    // It rejects every subcommand, and its listing prints nothing until
    // the host's listing window stops it: inconclusive, the host's.
    executable(
        &site_bin.join("archon-silent-tool"),
        "#!/bin/sh\ncase \"$1\" in\n  --list) exec /bin/sleep 30 ;;\nesac\necho \"error: unknown command '$1'\" >&2\nexit 1\n",
    );
    let (project, _outside, tasks, prd) = site_set(
        "archon-silent-tool item && test -f out.json",
        &format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", site_bin.display()),
        "/usr/bin:/bin".to_string(),
    );
    let attempt = |client: Arc<ScriptedAuthorJudge>| {
        let (project, tasks, prd) = (project.path(), &tasks, &prd);
        async move {
            freeze((project, tasks, prd), client)
                .await
                .expect_err("nothing unproven is published")
        }
    };
    let judged_repair = |client: &ScriptedAuthorJudge| {
        (client.judged_ids.lock().unwrap().iter())
            .filter(|id| id.as_str() == "AC-X-001")
            .count()
    };
    let first = Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, SOUND),
        |_, _| true,
    ));
    let error = attempt(first.clone()).await;
    assert!(
        crate::command::workflow_freeze_budget::FreezeIncomplete::caused(&error).is_some(),
        "resumable, not a failure: {error:#}"
    );
    assert!(format!("{error:#}").contains("AC-X-002"), "{error:#}");
    assert_eq!(
        first.authored(),
        1,
        "the authored finding went to its author"
    );
    let prompt = first.prompts.lock().unwrap()[0].clone();
    assert!(
        prompt.contains("AC-X-001")
            && prompt.contains(crate::command::workflow_task_set::executability::CANNOT_FAIL),
        "{prompt}"
    );
    // The next attempt: the saved repair is reused, never authored or
    // judged again (the draft's own whole-set judgment is all it asks).
    let second = Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, SOUND),
        |_, _| true,
    ));
    let error = attempt(second.clone()).await;
    HOST_PATH.with(|path| *path.borrow_mut() = None);
    assert!(
        crate::command::workflow_freeze_budget::FreezeIncomplete::caused(&error).is_some(),
        "{error:#}"
    );
    assert_eq!(second.authored(), 0, "the repair was kept");
    assert!(
        judged_repair(&second) < judged_repair(&first),
        "the repair is not judged again: first {:?} second {:?}",
        first.judged_ids.lock().unwrap(),
        second.judged_ids.lock().unwrap()
    );
}
