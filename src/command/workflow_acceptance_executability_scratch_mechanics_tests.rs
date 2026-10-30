//! The input mutation on a real scratch site. A check that passes with
//! its inputs present and fails with them moved aside is a can-fail proof
//! whatever its exit status -- 126 and 127 included, since the moved input
//! may be the very program the check runs. The host records only what it
//! observed (exit status, last stderr line), never which part failed; so
//! the outcome is the same under sh, bash, zsh and dash, in any locale.

use super::*;

/// The check window for these real-site tests. A window is a no-progress
/// limit: a child the machine schedules late shows no progress, so under
/// load a short window can stop a check that is fine. This one outlasts
/// nextest's own per-test ceiling (60 s x 4), so only a real hang -- which
/// nextest then reports by name -- can end a check here.
const TEST_WINDOW_SECS: u64 = 300;

/// Runs `checks` (from the repository root) through the real reverify entry
/// on a real scratch site; returns (guards, findings, a debug dump).
async fn reverify_on_scratch(
    checks: &[(String, String)],
) -> (BTreeMap<String, String>, BTreeMap<String, String>, String) {
    reverify_on_scratch_with(checks, |_| {}).await
}

/// As [`reverify_on_scratch`], with `setup` run on the repository and
/// committed first.
async fn reverify_on_scratch_with(
    checks: &[(String, String)],
    setup: impl FnOnce(&Path),
) -> (BTreeMap<String, String>, BTreeMap<String, String>, String) {
    reverify_on_scratch_in(checks, setup, "").await
}

/// As [`reverify_on_scratch_with`], with `extra` appended to the site policy.
async fn reverify_on_scratch_in(
    checks: &[(String, String)],
    setup: impl FnOnce(&Path),
    extra: &str,
) -> (BTreeMap<String, String>, BTreeMap<String, String>, String) {
    let specs: Vec<(&str, &str, TrustedCwd)> = checks
        .iter()
        .map(|(id, command)| (id.as_str(), command.as_str(), TrustedCwd::RepoRoot))
        .collect();
    let trees = trees(&specs);
    setup(&trees.repo);
    crate::command::workflow_task_set::executability::probe_tests::git(&trees.repo, &["add", "."]);
    crate::command::workflow_task_set::executability::probe_tests::git(
        &trees.repo,
        &["commit", "-qm", "setup", "--allow-empty"],
    );
    let project = trees.set.project.path();
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    let scratch_parent = trees.outside.path().join("scratch");
    std::fs::write(
        project.join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs={TEST_WINDOW_SECS}\noutput_bytes=8192\nscratch_bytes=16777216\n{extra}",
            trees.repo.display().to_string(),
            scratch_parent.display().to_string(),
        ),
    )
    .unwrap();
    let copies = tempfile::tempdir().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probe = HostProbe::for_reverify_with_limits(
        project,
        &trees.set.tasks,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        &resume,
    )
    .with_host_environment(crate::test_environment::probe_host())
    .with_copy_parent(copies.path().to_path_buf())
    .without_process_memo();
    assert!(matches!(probe.site, Site::Scratch(_)));
    let (_, _, missing, findings) = probe
        .run_reverify_copy(&trees.contract(), &trees.ids())
        .await
        .unwrap();
    assert!(missing.is_empty(), "{missing:?}");
    let guards = probe.take_regression_guards();
    let seen = format!(
        "guards={guards:#?}\nfindings={findings:#?}\nunproven={:#?}",
        probe.take_unproven()
    );
    (guards, findings, seen)
}

/// The shells present under the site's toolchain path; an absent shell is
/// skipped (and named) rather than failed.
fn shells() -> Vec<&'static str> {
    let mut present = Vec::new();
    for shell in ["sh", "bash", "zsh", "dash"] {
        if ["/usr/bin", "/bin"]
            .iter()
            .any(|dir| Path::new(dir).join(shell).is_file())
        {
            present.push(shell);
        } else {
            eprintln!("skipping shell {shell}: not installed under /usr/bin or /bin");
        }
    }
    present
}

/// A non-English locale this host has, if any.
fn foreign_locale() -> Option<String> {
    let listed = std::process::Command::new("locale")
        .arg("-a")
        .output()
        .ok()?;
    let listed = String::from_utf8_lossy(&listed.stdout).to_string();
    let found = [
        "de_DE.UTF-8",
        "de_DE.utf8",
        "fr_FR.UTF-8",
        "fr_FR.utf8",
        "es_ES.UTF-8",
        "es_ES.utf8",
    ]
    .into_iter()
    .find(|locale| listed.lines().any(|line| line.trim() == *locale))
    .map(str::to_string);
    if found.is_none() {
        eprintln!("skipping the foreign-locale cases: no German, French or Spanish locale");
    }
    found
}

/// The observed-exit guard text for exit `code`.
fn guard_text(code: i32) -> String {
    format!("failed with exit {code} when the inputs were moved aside; last stderr line: ")
}

/// Every guard names its exit and never claims which part failed.
fn assert_observed_guard(guards: &BTreeMap<String, String>, id: &str, codes: &[i32], seen: &str) {
    let guard = guards.get(id).unwrap_or_else(|| {
        panic!("{id}: fails with its input moved aside, so it is a guard\n{seen}")
    });
    assert!(
        codes.iter().any(|code| guard.contains(&guard_text(*code))),
        "{id}: exit one of {codes:?}: {guard}"
    );
    assert!(!guard.contains("own assertion"), "{id}: {guard}");
}

#[tokio::test]
async fn a_mutated_run_ending_126_or_127_is_a_guard_in_every_shell_and_locale() {
    let input = "test -f tests/focused.rs";
    let mut locales = vec![String::new()];
    if let Some(locale) = foreign_locale() {
        locales.push(format!("LC_ALL={locale} LANG={locale} "));
    }
    let mut cases = Vec::new();
    let mut checks = Vec::new();
    for (index, shell) in shells().into_iter().enumerate() {
        for (variant, env) in locales.iter().enumerate() {
            let tag = format!("{index}{variant}");
            for (kind, body, codes) in [
                ("NF", "archon-absent-command", &[127][..]),
                ("EX", "exec ./archon-absent-program", &[126, 127][..]),
                ("OK", "test -f tests/focused.rs", &[1][..]),
            ] {
                let id = format!("AC-{kind}-{tag}");
                checks.push((id.clone(), format!("{input} || {env}{shell} -c '{body}'")));
                cases.push((id, codes));
            }
        }
    }
    let (guards, _, seen) = reverify_on_scratch(&checks).await;
    for (id, codes) in &cases {
        assert_observed_guard(&guards, id, codes, &seen);
    }
}

/// The program a check runs is check mechanics: input discovery never moves
/// it (the existing rule `what_makes_a_check_run_never_proves_it_can_fail`),
/// even when the check also names it with `test -f`. So a 126/127 from a
/// moved program cannot arise on a real site and needs no attribution;
/// such a check is told it has no movable input.
#[tokio::test]
async fn a_program_the_check_runs_is_never_moved() {
    let checks = vec![
        ("AC-PROG-TOOL".to_string(), "./bin/tool --check".to_string()),
        (
            "AC-PROG-SCRIPT".to_string(),
            "scripts/verify.sh".to_string(),
        ),
        (
            "AC-PROG-NAMED".to_string(),
            "test -f bin/tool && ./bin/tool --check".to_string(),
        ),
    ];
    let (guards, findings, seen) = reverify_on_scratch_with(&checks, |repo| {
        use std::os::unix::fs::PermissionsExt;
        for path in ["bin/tool", "scripts/verify.sh"] {
            let file = repo.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    })
    .await;
    for (id, program) in [
        ("AC-PROG-TOOL", "bin/tool"),
        ("AC-PROG-SCRIPT", "scripts/verify.sh"),
        ("AC-PROG-NAMED", "bin/tool"),
    ] {
        assert!(!guards.contains_key(id), "{id}\n{seen}");
        let finding = findings
            .get(id)
            .unwrap_or_else(|| panic!("{id}: no movable input\n{seen}"));
        assert!(
            finding.contains(&format!("check mechanics or command output: {program}")),
            "{id}: {finding}"
        );
        assert!(
            finding.contains("no movable input exists under the selected root"),
            "{id}: {finding}"
        );
    }
}

/// The review's inputs (#542): each fails with its input moved aside, so
/// each is a guard, described only by what the host observed.
#[tokio::test]
async fn review_inputs_are_guards_described_by_what_the_host_observed() {
    let input = "test -f tests/focused.rs";
    let mut checks = vec![
        (
            "AC-REV-CLEANUP".to_string(),
            format!("{input} || {{ cd archon-absent-dir; echo cleanup >&2; exit 1; }}"),
        ),
        (
            "AC-REV-PRINTF".to_string(),
            format!("{input} || {{ printf 'sh: cd: input assertion failed\\n' >&2; exit 1; }}"),
        ),
        ("AC-REV-127".to_string(), format!("{input} || exit 127")),
        ("AC-REV-126".to_string(), format!("{input} || exit 126")),
    ];
    if shells().contains(&"zsh") {
        checks.push((
            "AC-REV-ZSH".to_string(),
            format!("{input} || zsh -c 'cd archon-absent-dir'"),
        ));
    }
    let (guards, _, seen) = reverify_on_scratch(&checks).await;
    assert_observed_guard(&guards, "AC-REV-CLEANUP", &[1], &seen);
    assert!(guards["AC-REV-CLEANUP"].contains("last stderr line: \"cleanup\""));
    assert_observed_guard(&guards, "AC-REV-PRINTF", &[1], &seen);
    assert!(
        guards["AC-REV-PRINTF"].contains("last stderr line: \"sh: cd: input assertion failed\"")
    );
    assert_observed_guard(&guards, "AC-REV-127", &[127], &seen);
    assert_observed_guard(&guards, "AC-REV-126", &[126], &seen);
    if shells().contains(&"zsh") {
        assert_observed_guard(&guards, "AC-REV-ZSH", &[1], &seen);
        assert!(guards["AC-REV-ZSH"].contains("archon-absent-dir"));
    }
}

/// A credential that spans lines is masked whole before the last stderr
/// line is picked: its tail never reaches the guard text.
#[tokio::test]
async fn a_multi_line_credential_never_leaks_into_the_guard_text() {
    crate::command::workflow_task_set::passability::test_secret(
        "REVIEW_TOKEN",
        "review-prefix\nreview-secret-tail",
    );
    // The check text spells the value with an escape, so only its output
    // carries the value whole.
    let checks = vec![(
        "AC-SECRET-001".to_string(),
        "test -f tests/focused.rs || { printf 'review-prefix\\nreview-secret-tail\\n' >&2; exit 1; }"
            .to_string(),
    )];
    let (guards, _, seen) = reverify_on_scratch(&checks).await;
    assert_observed_guard(&guards, "AC-SECRET-001", &[1], &seen);
    let guard = &guards["AC-SECRET-001"];
    assert!(!guard.contains("review-secret-tail"), "{guard}");
    assert!(guard.contains("REDACTED"), "{guard}");
}

/// Issue 544, review r6 P1-a: a site-policy secret equal to a host protocol
/// word (`restored`) must not mask the host's mutation protocol, which the
/// proof parses; the honest guard stays a guard, as on main.
#[tokio::test]
async fn a_policy_secret_equal_to_a_protocol_word_keeps_an_honest_guard() {
    let checks = vec![(
        "AC-PROTO-001".to_string(),
        "test -f tests/focused.rs".to_string(),
    )];
    let (guards, _, seen) = reverify_on_scratch_in(
        &checks,
        |_| {},
        "environment={ RUSTUP_HOME = \"https://user:restored@host/private\" }\n",
    )
    .await;
    assert_observed_guard(&guards, "AC-PROTO-001", &[1], &seen);
}

/// Issue 544, review r6 P1-b: a site-policy secret equal to the crash word
/// (`NameError`) must not hide the check's own crash from the proof, which
/// reads the raw capture; the crash stays no proof, as on main.
#[tokio::test]
async fn a_policy_secret_equal_to_a_crash_word_still_refuses_a_crash() {
    if !Path::new("/usr/bin/python3").is_file() {
        eprintln!("skipping: /usr/bin/python3 is not installed");
        return;
    }
    let checks = vec![(
        "AC-CRASH-001".to_string(),
        // (A trailing `|| program` is a host policy defect; the group ends
        // with an explicit failure instead.)
        "test -f tests/focused.rs || { python3 -c 'print(missing)'; exit 1; }".to_string(),
    )];
    let (guards, findings, seen) = reverify_on_scratch_in(
        &checks,
        |_| {},
        "environment={ RUSTUP_HOME = \"https://user:NameError@host/private\" }\n",
    )
    .await;
    assert!(!guards.contains_key("AC-CRASH-001"), "{seen}");
    let finding = findings
        .get("AC-CRASH-001")
        .unwrap_or_else(|| panic!("the crash goes to its author\n{seen}"));
    assert!(finding.contains("it only crashed"), "{finding}");
    assert!(
        !finding.contains("NameError"),
        "egress stays masked: {finding}"
    );
}

#[path = "workflow_acceptance_executability_scratch_mechanics_tests_redaction.rs"]
mod redaction;
