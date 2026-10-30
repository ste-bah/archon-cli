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

/// Every stored output stream (the `stdout`/`stderr` of a stored result,
/// in an observation, a reused verdict or an output file) under `roots`
/// that holds `needle`. Path names a store records (inventories, check
/// text) are the repository's, not output, and are not searched.
fn streams_holding(roots: &[&Path], needle: &str) -> Vec<String> {
    fn streams(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, item) in map {
                    if key == "stdout" || key == "stderr" {
                        match item {
                            serde_json::Value::String(text) => out.push(text.clone()),
                            serde_json::Value::Array(bytes) => out.push(
                                String::from_utf8_lossy(
                                    &bytes
                                        .iter()
                                        .filter_map(|byte| byte.as_u64().map(|b| b as u8))
                                        .collect::<Vec<_>>(),
                                )
                                .into_owned(),
                            ),
                            other => streams(other, out),
                        }
                    } else {
                        streams(item, out);
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|item| streams(item, out)),
            _ => {}
        }
    }
    fn visit(dir: &Path, needle: &str, found: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                continue;
            }
            if path.is_dir() {
                visit(&path, needle, found);
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if name.ends_with(".stdout") || name.ends_with(".stderr") {
                if String::from_utf8_lossy(&bytes).contains(needle) {
                    found.push(path.display().to_string());
                }
            } else if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let mut texts = Vec::new();
                streams(&value, &mut texts);
                if texts.iter().any(|text| text.contains(needle)) {
                    found.push(path.display().to_string());
                }
            }
        }
    }
    let mut found = Vec::new();
    for root in roots {
        visit(root, needle, &mut found);
    }
    found
}

/// Review r7 P1-1: a host protocol line that names a moved path holding a
/// policy secret (`covered by moved ancestor '<path>'`) is masked like any
/// other line: no stored observation or reused verdict keeps the secret,
/// and the honest guard is still a guard.
#[tokio::test]
async fn a_host_protocol_path_holding_a_policy_secret_is_masked_everywhere() {
    const PASSWORD: &str = "violet-orbit-94";
    let command = format!("test -f tests/{PASSWORD}/item && test -d tests/{PASSWORD}");
    let trees = trees(&[("AC-PATH-001", command.as_str(), TrustedCwd::RepoRoot)]);
    std::fs::create_dir_all(trees.repo.join(format!("tests/{PASSWORD}"))).unwrap();
    std::fs::write(trees.repo.join(format!("tests/{PASSWORD}/item")), "present").unwrap();
    crate::command::workflow_task_set::executability::probe_tests::git(&trees.repo, &["add", "."]);
    crate::command::workflow_task_set::executability::probe_tests::git(
        &trees.repo,
        &["commit", "-qm", "secret-named input"],
    );
    let project = trees.set.project.path();
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    std::fs::write(
        project.join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs={TEST_WINDOW_SECS}\noutput_bytes=8192\nscratch_bytes=16777216\nenvironment={{ RUSTUP_HOME = \"https://user:{PASSWORD}@host/private\" }}\n",
            trees.repo.display().to_string(),
            trees.outside.path().join("scratch").display().to_string(),
        ),
    )
    .unwrap();
    let copies = tempfile::tempdir().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::saving(
        crate::command::workflow_freeze_budget::FreezeBudget::unlimited(),
        true,
    );
    let probe = HostProbe::for_reverify_with_limits(
        project,
        &trees.set.tasks,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
        &resume,
    )
    .with_host_environment(crate::test_environment::probe_host())
    .with_copy_parent(copies.path().to_path_buf());
    let (_, _, missing, findings) = probe
        .run_reverify_copy(&trees.contract(), &trees.ids())
        .await
        .unwrap();
    assert!(missing.is_empty(), "{missing:?}");
    let guards = probe.take_regression_guards();
    let seen = format!("guards={guards:#?}\nfindings={findings:#?}");
    // The honest guard is still a guard.
    assert_observed_guard(&guards, "AC-PATH-001", &[1], &seen);
    let roots = [
        trees.outside.path().join("scratch"),
        copies.path().to_path_buf(),
        project.to_path_buf(),
    ];
    let roots: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
    let leaking = streams_holding(&roots, PASSWORD);
    assert!(
        leaking.is_empty(),
        "stored output streams leak it: {leaking:?}"
    );
}

/// Review r7 P1-2: a check the shell refuses to start (`dash -Z`, exit 2)
/// gives no verdict even when a policy secret equals the shell's refusal
/// words: the verdict is decided from the raw capture, and a result whose
/// decisive text masking changed is never reused from its masked copy.
#[tokio::test]
async fn a_policy_secret_equal_to_refusal_words_keeps_the_refusal_on_reuse() {
    if !["/bin/dash", "/usr/bin/dash"]
        .iter()
        .any(|path| Path::new(path).is_file())
    {
        eprintln!("skipping: dash is not installed");
        return;
    }
    let trees = trees(&[("AC-REFUSE-001", "dash -Z", TrustedCwd::RepoRoot)]);
    let project = trees.set.project.path();
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    std::fs::write(
        project.join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs={TEST_WINDOW_SECS}\noutput_bytes=8192\nscratch_bytes=16777216\nenvironment={{ RUSTUP_HOME = \"https://u:Illegal%20option@host/private\" }}\n",
            trees.repo.display().to_string(),
            trees.outside.path().join("scratch").display().to_string(),
        ),
    )
    .unwrap();
    let copies = tempfile::tempdir().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::saving(
        crate::command::workflow_freeze_budget::FreezeBudget::unlimited(),
        true,
    );
    let mut texts = Vec::new();
    for _ in 0..2 {
        // A freeze probe in a new process: verdicts come only from disk.
        let probe = HostProbe::for_task_set(project, &trees.set.tasks)
            .with_host_environment(crate::test_environment::probe_host())
            .with_copy_parent(copies.path().to_path_buf())
            .with_resume(&resume)
            .without_process_memo();
        let findings = probe.script_defects(&trees.contract(), &trees.ids()).await;
        let unproven = probe.take_unproven();
        texts.push(format!("findings={findings:?} unproven={unproven:?}"));
    }
    for (run, text) in texts.iter().enumerate() {
        assert!(
            text.contains("rejected an invocation option"),
            "run {run} lost the shell's refusal: {text}"
        );
        assert!(
            !text.contains("Illegal option"),
            "egress stays masked: {text}"
        );
    }
}

/// Review r8: a mutation run that crashed (its own `NameError`) is refused
/// on a cached rerun as on the first run, even when a policy secret equals
/// the crash word: the reused copy never decides from masked stderr.
async fn cached_crash_runs(environment: &str) {
    if !Path::new("/usr/bin/python3").is_file() {
        eprintln!("skipping: /usr/bin/python3 is not installed");
        return;
    }
    let command = "test -f tests/focused.rs || { python3 -c 'print(missing)'; exit 1; }";
    let trees = trees(&[("AC-CRASH-002", command, TrustedCwd::RepoRoot)]);
    let project = trees.set.project.path();
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    std::fs::write(
        project.join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs={TEST_WINDOW_SECS}\noutput_bytes=8192\nscratch_bytes=16777216\n{environment}",
            trees.repo.display().to_string(),
            trees.outside.path().join("scratch").display().to_string(),
        ),
    )
    .unwrap();
    let copies = tempfile::tempdir().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::saving(
        crate::command::workflow_freeze_budget::FreezeBudget::unlimited(),
        true,
    );
    let mut outcomes = Vec::new();
    for _ in 0..2 {
        // A probe in a new process: a reused verdict comes only from disk.
        let probe = HostProbe::for_reverify_with_limits(
            project,
            &trees.set.tasks,
            archon_tools::workflow_resource_env::CargoResourceLimits::default(),
            &resume,
        )
        .with_host_environment(crate::test_environment::probe_host())
        .with_copy_parent(copies.path().to_path_buf())
        .without_process_memo();
        let (_, _, missing, findings) = probe
            .run_reverify_copy(&trees.contract(), &trees.ids())
            .await
            .unwrap();
        assert!(missing.is_empty(), "{missing:?}");
        outcomes.push((probe.take_regression_guards(), findings));
    }
    for (run, (guards, findings)) in outcomes.iter().enumerate() {
        assert!(
            guards.is_empty(),
            "run {run} kept a crash as a guard: {guards:?}"
        );
        assert!(
            findings
                .get("AC-CRASH-002")
                .is_some_and(|finding| finding.contains("it only crashed")),
            "run {run}: {findings:?}"
        );
    }
}

#[tokio::test]
async fn a_cached_mutation_crash_is_refused_as_on_the_first_run() {
    cached_crash_runs("environment={ RUSTUP_HOME = \"https://user:NameError@host/private\" }\n")
        .await;
}

/// Review r9: the result store masks more than the capture does (an
/// unforwarded host credential such as `REVIEW_SESSION=NameError`); the
/// reuse rule is judged on the exact bytes the store persists, so the
/// cached rerun still refuses the crash.
#[tokio::test]
async fn a_crash_word_masked_only_by_the_store_is_refused_on_reuse() {
    crate::command::workflow_task_set::passability::test_secret("REVIEW_SESSION", "NameError");
    cached_crash_runs("").await;
}
