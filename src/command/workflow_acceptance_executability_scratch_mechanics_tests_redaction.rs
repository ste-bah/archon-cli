use super::*;

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
