//! Operator-only replay. All criterion identities and text arrive at runtime.
use super::*;
use std::ffi::OsStr;

fn required(name: &str) -> PathBuf {
    std::env::var_os(name).map(PathBuf::from).expect(name)
}

// Never echo command arguments, config contents, or child streams on failure.
fn git_at(repository: &Path, args: &[&OsStr]) -> String {
    let output = std::process::Command::new("git")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(["-c", "core.hooksPath=/dev/null", "-C"])
        .arg(repository)
        .args(args)
        .output()
        .map_err(|_| ())
        .expect("offline git invocation failed");
    assert!(output.status.success(), "offline git invocation failed");
    String::from_utf8(output.stdout)
        .map_err(|_| ())
        .expect("git output was not UTF-8")
        .trim()
        .to_owned()
}

#[tokio::test]
#[ignore = "operator only: real base-tree builds; run alone with runtime inputs"]
async fn offline_acceptance_contract_replay() {
    let source = required("R16_REPO")
        .canonicalize()
        .expect("repository unavailable");
    let config_path = required("R16_PROJECT_CONFIG");
    let config_text = std::fs::read_to_string(&config_path)
        .map_err(|_| ())
        .expect("project config unavailable");
    let document: toml::Value = toml::from_str(&config_text)
        .map_err(|_| ())
        .expect("project config is not TOML");
    let config: archon_core::config::AcceptanceExecutionConfig = document
        .get("workflow")
        .and_then(|v| v.get("acceptance_execution"))
        .expect("acceptance execution policy required")
        .clone()
        .try_into()
        .map_err(|_| ())
        .expect("invalid acceptance execution policy");
    assert!(
        config.repository.canonicalize().is_ok_and(|p| p == source),
        "R16_REPO must match the configured repository"
    );
    let config_parent = config_path.parent().expect("config parent required");
    let source_project = if config_parent.file_name() == Some(OsStr::new(".archon")) {
        config_parent.parent().expect("project parent required")
    } else {
        config_parent
    }
    .canonicalize()
    .expect("project unavailable");
    let mut contract: AcceptanceContract = serde_json::from_slice(
        &std::fs::read(required("R16_CONTRACT"))
            .map_err(|_| ())
            .expect("contract unavailable"),
    )
    .map_err(|_| ())
    .expect("invalid acceptance contract");
    let requested = std::env::var("R16_IDS").expect("R16_IDS required");
    let ids: BTreeSet<String> = requested.split_whitespace().map(str::to_owned).collect();
    assert!(!ids.is_empty(), "select at least one entry");
    for id in &ids {
        assert_eq!(
            contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .filter(|e| &e.id == id)
                .count(),
            1,
            "entry must occur exactly once: {id}"
        );
    }
    contract.acceptance.retain(|e| ids.contains(&e.id));
    contract.supplementary.retain(|e| ids.contains(&e.id));
    // refs_for requires Accepted to select a command; resolve_command also
    // requires nonempty judgment fields across the whole artifact. These are
    // fixture admission metadata, not claims about passability. The resolver
    // uses only their presence and the verdict, never this prose; script_defects
    // still computes every executability finding from the unchanged checks.
    for entry in contract
        .acceptance
        .iter_mut()
        .chain(&mut contract.supplementary)
    {
        entry.judgment = archon_workflow::task_set_contract::JudgeVerdict {
            verdict: JudgeDecision::Accepted,
            counterexample: "Not evaluated: offline executability fixture calls no judge.".into(),
            reason: "Offline executability fixture; no judge called".into(),
            host_call_id: "offline-executability-replay".into(),
            sampling: None,
        };
    }
    assert!(
        contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .all(|e| matches!(e.check, AcceptanceCheck::Command { .. })),
        "command entries required"
    );
    // Exercise the scratch runner's actual authorization boundary, without
    // launching a shell/check/cargo. Do this in real mode too, before cloning.
    // Use the same whole contract and refs as HostProbe, so another entry's
    // malformed judgment cannot hide behind per-entry selection again.
    let digest = super::super::contract_digest(&contract).expect("fixture digest required");
    let refs = super::super::refs_for(&contract, &digest, &ids);
    assert_eq!(refs.len(), ids.len(), "every selected entry must be probed");
    for reference in &refs {
        archon_workflow::acceptance_world::resolve_command(&contract, &digest, reference)
            .unwrap_or_else(|error| {
                panic!(
                    "fixture authorization failed for {}: {error}",
                    reference.acceptance_id
                )
            });
        println!("artifact_validation: {}: passed", reference.acceptance_id);
    }
    let source_forms = crate::command::workflow_task_set::live_root::root_forms([
        source.as_path(),
        source_project.as_path(),
    ]);
    for entry in contract.acceptance.iter().chain(&contract.supplementary) {
        if let AcceptanceCheck::Command { command, .. } = &entry.check {
            assert!(
                crate::command::workflow_task_set::live_root::named_root(command, &source_forms)
                    .is_none(),
                "a selected command names a live source root; refusing to execute it"
            );
        }
    }

    let parent = required("R16_SCRATCH");
    let parent = parent.canonicalize().expect("cannot resolve replay parent");
    assert!(
        !parent.starts_with(&source) && !parent.starts_with(&source_project),
        "replay parent must be outside both source roots"
    );
    let run = tempfile::Builder::new()
        .prefix("offline-acceptance-")
        .tempdir_in(&parent)
        .expect("cannot create replay directory");
    // Preserve the repository basename: literal ../<repository>/ aliases matter.
    let repository = run
        .path()
        .join(source.file_name().expect("repository basename required"));
    let project = run.path().join("project-copy");
    assert_ne!(
        repository, project,
        "repository basename collides with project copy"
    );
    std::fs::create_dir_all(&repository).expect("cannot create repository copy");
    let base = git_at(
        &source,
        &["rev-parse".as_ref(), "6cd0a28^{commit}".as_ref()],
    );
    std::fs::create_dir_all(&project).expect("cannot create project copy");
    let tasks = project.join("replay-tasks");
    std::fs::create_dir_all(&tasks).expect("cannot create copied task root");
    let policy = archon_workflow::acceptance_scratch::ScratchPolicy {
        repository: repository.clone(),
        project: project.clone(),
        task_root: tasks,
        scratch_parent: run.path().join("sites"),
        project_inputs: config.project_inputs,
        project_input_excludes: config.project_input_excludes,
        combined: matches!(
            config.project_repository_view,
            archon_core::config::AcceptanceProjectView::Combined
        ),
        toolchain_path: config.toolchain_path,
        environment: config.environment,
        environment_allowlist: config.environment_allowlist,
        cargo_seed: config.cargo_seed,
        timeout_secs: config.timeout_secs,
        output_bytes: config.output_bytes,
        scratch_bytes: config.scratch_bytes,
        build_cache: None,
    };
    policy
        .validate()
        .unwrap_or_else(|error| panic!("invalid relocated scratch policy: {error}"));
    let binding = NativeBinding {
        policy,
        source_commit: base.clone(),
        external_data_roots: Vec::new(),
    }
    .with_run_build_cache("offline-replay");
    binding
        .policy
        .validate()
        .unwrap_or_else(|error| panic!("invalid cached scratch policy: {error}"));
    assert!(
        base.len() == 40 && base.bytes().all(|b| b.is_ascii_hexdigit()),
        "clone plan requires a resolved base commit"
    );
    assert!(binding.policy.task_root.starts_with(&project));
    for path in [
        &repository,
        &project,
        &binding.policy.task_root,
        &binding.policy.scratch_parent,
        binding
            .policy
            .build_cache
            .as_ref()
            .expect("build cache required"),
        &run.path().join("copies"),
    ] {
        assert!(
            path.starts_with(run.path()),
            "clone plan escaped owned scratch"
        );
    }
    if let Some(seed) = &binding.policy.cargo_seed {
        assert!(seed.is_dir(), "configured cargo seed unavailable");
    }
    // Cargo reads ancestor configuration from every relocated check cwd. This
    // enforces offline mode without adding an illegal static environment binding,
    // changing literal check commands, or replacing the real scratch runner.
    let offline_config = run.path().join(".cargo");
    std::fs::create_dir(&offline_config).expect("cannot create offline config directory");
    std::fs::write(
        offline_config.join("config.toml"),
        "[net]\noffline = true\n",
    )
    .expect("cannot write offline cargo config");
    if std::env::var_os("R16_DRY").as_deref() == Some(OsStr::new("1")) {
        println!("dry_run: valid relocated policy and scratch clone plan");
        println!("scratch_root: {}", run.path().display());
        println!("repository_copy: {}", repository.display());
        println!("project_copy: {}", project.display());
        println!("scratch_sites: {}", binding.policy.scratch_parent.display());
        println!(
            "entries: {}; project_inputs: {}; exclusions: {}; combined: {}; cargo_seed: {}",
            ids.len(),
            binding.policy.project_inputs.len(),
            binding.policy.project_input_excludes.len(),
            binding.policy.combined,
            binding.policy.cargo_seed.is_some()
        );
        println!(
            "clone_plan: independent local bare clone, no hardlinks, remove origin, materialize resolved base"
        );
        println!(
            "policy: configured toolchain, bindings, allowlist, seed and limits retained; offline cargo ancestor config"
        );
        println!("dry_run: stopped before cloning, input copying, check processes or child cargo");
        return;
    }
    // The ONLY git operations against the source are rev-parse and this local
    // clone. No shared objects/alternates, source worktrees, or source index writes.
    git_at(
        run.path(),
        &[
            "clone".as_ref(),
            "--quiet".as_ref(),
            "--bare".as_ref(),
            "--no-hardlinks".as_ref(),
            "--".as_ref(),
            source.as_os_str(),
            repository.join(".git").as_os_str(),
        ],
    );
    git_at(
        &repository,
        &["config".as_ref(), "core.bare".as_ref(), "false".as_ref()],
    );
    git_at(
        &repository,
        &["remote".as_ref(), "remove".as_ref(), "origin".as_ref()],
    );
    git_at(
        &repository,
        &[
            "symbolic-ref".as_ref(),
            "HEAD".as_ref(),
            "refs/heads/offline-replay".as_ref(),
        ],
    );
    git_at(
        &repository,
        &[
            "update-ref".as_ref(),
            "refs/heads/offline-replay".as_ref(),
            base.as_ref(),
        ],
    );
    git_at(
        &repository,
        &[
            "read-tree".as_ref(),
            "-m".as_ref(),
            "-u".as_ref(),
            "HEAD".as_ref(),
        ],
    );
    assert_eq!(
        git_at(&repository, &["rev-parse".as_ref(), "HEAD".as_ref()]),
        base
    );
    let policy = binding.policy.clone();
    let mut remaining = PROJECT_COPY_BYTES;
    for input in &policy.project_inputs {
        let before = data_digest_scoped(
            &source_project,
            &source,
            None,
            input,
            &policy.project_input_excludes,
        )
        .map_err(|_| ())
        .expect("cannot digest project input");
        if let Some(dir) = project.join(input).parent() {
            std::fs::create_dir_all(dir).expect("cannot create input parent");
        }
        copy_data(
            &source_project,
            input,
            &project,
            &source,
            None,
            &mut remaining,
        )
        .map_err(|_| ())
        .expect("cannot copy project input");
        assert!(
            data_digest_scoped(
                &source_project,
                &source,
                None,
                input,
                &policy.project_input_excludes
            )
            .is_ok_and(|after| after == before),
            "project input changed while copying"
        );
    }
    // Requirement-aware deterministic findings need the PRD, without an absolute
    // source path remaining in the probe's contract.
    let prd = std::fs::read(source_project.join(&contract.prd.path))
        .map_err(|_| ())
        .expect("contract PRD unavailable");
    contract.prd.path = "replay-prd.md".into();
    std::fs::write(project.join(&contract.prd.path), prd).expect("cannot copy PRD");
    // Same ExecutabilityProbe entry as freeze-acceptance, with no judge invocation.
    for id in &ids {
        let probe = HostProbe::new(
            project.clone(),
            repository.clone(),
            Site::Scratch(Box::new(binding.clone())),
        )
        .with_baseline(Baseline {
            origin: super::super::BaselineOrigin::RecordedBase,
            commit: base.clone(),
            repository: repository.clone(),
        })
        .with_copy_parent(run.path().join("copies"));
        let findings = probe
            .script_defects(&contract, &BTreeSet::from([id.clone()]))
            .await;
        let guards = probe.take_regression_guards();
        let unproven = probe.take_unproven_causes();
        let class = if findings.contains_key(id) {
            "finding"
        } else if unproven.contains_key(id) {
            "host_unproven"
        } else if guards.contains_key(id) {
            "regression_guard"
        } else {
            "no_finding"
        };
        println!("entry: {id}\nverdict_class: {class}");
        for text in findings.values() {
            println!("finding:\n{text}");
        }
        for text in guards.values() {
            println!("guard_note:\n{text}");
        }
        for (text, cause) in unproven.values() {
            println!("host_unproven ({}):\n{text}", cause.code());
        }
        for text in probe.take_diagnostics() {
            println!("note:\n{text}");
        }
        println!("end_entry: {id}");
    }
    // No PRD-specific verdict assertions: the operator compares the complete text.
}
