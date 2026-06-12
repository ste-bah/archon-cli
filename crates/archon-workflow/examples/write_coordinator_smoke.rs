//! Live smoke for PRD-012 TASK-WC-001: config parse -> runtime resolve -> spec guard.
//!
//! Exits non-zero on any mismatch. Run:
//! `cargo run -p archon-workflow --example write_coordinator_smoke -- <git-root>`

use std::path::Path;
use std::process::exit;

use archon_workflow::WorkflowConfig;
use archon_workflow::spec::WorkflowSpec;
use archon_workflow::write_coordinator::{
    SerialFallbackReason, WriteCoordinatorRuntime, resolve_write_coordinator_runtime,
};

fn main() {
    let git_root = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());

    let toml_src = "[write_coordinator]\nmax_patch_bytes = 2048\n";
    let cfg: WorkflowConfig = match toml::from_str(toml_src) {
        Ok(cfg) => cfg,
        Err(err) => fail(&format!("config TOML rejected: {err}")),
    };
    let wc = cfg.write_coordinator;
    println!(
        "config parsed: enabled={} max_patch_bytes={} fail_on_undeclared_write={}",
        wc.enabled, wc.max_patch_bytes, wc.fail_on_undeclared_write
    );
    if !wc.enabled || wc.max_patch_bytes != 2048 {
        fail("config defaults/overrides wrong");
    }

    match resolve_write_coordinator_runtime(Path::new(&git_root), &wc) {
        WriteCoordinatorRuntime::Enabled { canonical_root } => {
            println!("runtime resolved: Enabled at {}", canonical_root.display());
        }
        other => fail(&format!("expected Enabled for {git_root}, got {other:?}")),
    }

    let non_git = std::env::temp_dir();
    match resolve_write_coordinator_runtime(&non_git, &wc) {
        WriteCoordinatorRuntime::Disabled {
            reason: SerialFallbackReason::NonGitRoot,
        } => println!(
            "runtime resolved: Disabled(NonGitRoot) for {}",
            non_git.display()
        ),
        other => fail(&format!("expected Disabled(NonGitRoot), got {other:?}")),
    }

    let bad_yaml = r#"
schema: archon.workflow.v1
name: smoke
task: smoke write coordination
stages:
  - id: impl
    kind: fanout
    item_kind: implementation
    input:
      items:
        - name: undeclared
"#;
    let spec = WorkflowSpec::from_yaml(bad_yaml).unwrap_or_else(|err| {
        fail(&format!("base spec rejected: {err}"));
    });
    match spec.validate_write_coordination(&wc) {
        Err(err) => println!("spec guard fired as designed: {err}"),
        Ok(()) => fail("spec guard MISSED an undeclared-target implementation fanout"),
    }

    // TASK-WC-002 — path normalization + resource keys + provenance.
    use archon_workflow::write_coordinator::write_plan::{
        ResourceKey, TargetFilesSource, keys_conflict, normalize_target, parse_baseline_id,
        resolve_target_files,
    };
    let root = Path::new(&git_root);
    let n = normalize_target("crates\\archon-workflow/src/lib.rs", root)
        .unwrap_or_else(|err| fail(&format!("normalize_target rejected real file: {err}")));
    println!("normalized: {}", n.as_str());
    if n.as_str() != "crates/archon-workflow/src/lib.rs" {
        fail("backslash normalization wrong");
    }
    if normalize_target("../../etc/passwd", root).is_ok() {
        fail("traversal escape was NOT rejected");
    }
    let (files, source) = resolve_target_files(
        &serde_json::json!({"target_files": ["src/a.rs"]}),
        &["fallback.rs".to_string()],
    )
    .unwrap_or_else(|err| fail(&format!("resolve_target_files failed: {err}")));
    if source != TargetFilesSource::Item || files != ["src/a.rs"] {
        fail("provenance resolution wrong");
    }
    if !keys_conflict(
        &ResourceKey::File("a/b.rs".into()),
        &ResourceKey::Dir("a".into()),
    ) {
        fail("file-under-dir conflict not detected");
    }
    parse_baseline_id("blake3:deadbeef")
        .unwrap_or_else(|err| fail(&format!("baseline id rejected: {err}")));
    println!("write_plan smoke: provenance={source:?}, conflict-detection OK");

    smoke_worktree_isolation();
    smoke_conflict_graph(root);
    smoke_patch_manifest();
    smoke_patch_apply();

    println!("write_coordinator smoke: OK");
}

/// TASK-WC-006 — apply a captured patch to canonical under the repo write lock.
fn smoke_patch_apply() {
    use archon_workflow::WriteCoordinatorConfig;
    use archon_workflow::write_coordinator::patch_apply::{
        ApplyResumeStatus, apply_wave, resume_status, run_wave_verify, with_repo_lock,
    };
    use archon_workflow::write_coordinator::patch_manifest::{
        ManifestStatus, PatchManifest, capture_patch, persist_manifest,
    };
    use archon_workflow::write_coordinator::worktree_isolation::{
        capture_canonical_baseline, create_item_workspace,
    };
    use archon_workflow::write_coordinator::write_plan::{TargetFilesSource, normalize_target};
    use archon_workflow::write_coordinator::{ItemId, WritePlan};
    use std::collections::BTreeMap;

    fn git(args: &[&str], cwd: &Path) {
        let out = std::process::Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap_or_else(|e| fail(&format!("git spawn: {e}")));
        if !out.status.success() {
            fail(&format!(
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }

    let dir = std::env::temp_dir().join(format!("wc-apply-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let root = dir.as_path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "user.name", "smoke"], root);
    git(&["config", "user.email", "smoke@local"], root);
    std::fs::write(root.join("src/lib.rs"), "// committed\n").unwrap();
    git(&["add", "src/lib.rs"], root);
    git(&["commit", "-q", "-m", "init"], root);

    let plan = WritePlan {
        run_id: "run1".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from("impl-0"),
        canonical_root: root.to_path_buf(),
        isolated_root: root.join(".archon/wc/run1/impl-0"),
        target_files: vec![normalize_target("src/lib.rs", root).unwrap()],
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: Default::default(),
    };
    let cfg = WriteCoordinatorConfig::default();
    let baseline = capture_canonical_baseline(root, &plan, &[], &cfg)
        .unwrap_or_else(|e| fail(&format!("baseline: {e}")));
    let ws = create_item_workspace(root, &plan, &baseline)
        .unwrap_or_else(|e| fail(&format!("workspace: {e}")));
    std::fs::write(plan.isolated_root.join("src/lib.rs"), "// applied\n").unwrap();
    let captured = capture_patch(&ws, &plan.target_files, &baseline)
        .unwrap_or_else(|e| fail(&format!("capture: {e}")));
    let run_root = root.join(".archon/workflows/run1");
    persist_manifest(
        &run_root,
        "run1",
        "impl",
        &plan.item_id,
        &captured,
        ManifestStatus::PendingApply,
    )
    .unwrap_or_else(|e| fail(&format!("persist: {e}")));
    let json = std::fs::read_to_string(
        run_root.join("write-coordination/stages/impl/manifests/impl-0.json"),
    )
    .unwrap();
    let manifest: PatchManifest = serde_json::from_str(&json).unwrap();
    let mut pre = BTreeMap::new();
    pre.insert(plan.item_id.clone(), captured.pre_hashes.clone());

    let result = with_repo_lock(root, || {
        let rec = apply_wave(root, &[manifest], &pre, 0, &run_root, "run1", "impl")?;
        let verify = run_wave_verify(root, Some("true"), 0, &run_root, "impl")?;
        Ok((rec, verify))
    })
    .unwrap_or_else(|e| fail(&format!("apply under lock: {e}")));
    let (rec, verify) = result;
    if rec.items_applied != ["impl-0"] {
        fail(&format!(
            "unexpected items_applied: {:?}",
            rec.items_applied
        ));
    }
    if std::fs::read_to_string(root.join("src/lib.rs")).unwrap() != "// applied\n" {
        fail("patch was not applied to canonical");
    }
    if verify.exit != 0 {
        fail("verify command failed");
    }
    if resume_status(&plan.item_id, &run_root, "impl") != ApplyResumeStatus::Applied {
        fail("resume_status not Applied after apply");
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!("patch_apply smoke: applied under lock, verify exit 0, resume=Applied");
}

/// TASK-WC-005 — capture a real patch, validate it, and persist the manifest.
fn smoke_patch_manifest() {
    use archon_workflow::WriteCoordinatorConfig;
    use archon_workflow::write_coordinator::patch_manifest::{
        ManifestStatus, capture_patch, persist_manifest, validate_patch,
    };
    use archon_workflow::write_coordinator::worktree_isolation::{
        capture_canonical_baseline, create_item_workspace,
    };
    use archon_workflow::write_coordinator::write_plan::{TargetFilesSource, normalize_target};
    use archon_workflow::write_coordinator::{ItemId, WritePlan};
    use std::collections::BTreeSet;

    fn git(args: &[&str], cwd: &Path) {
        let out = std::process::Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap_or_else(|e| fail(&format!("git spawn: {e}")));
        if !out.status.success() {
            fail(&format!(
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }

    let dir = std::env::temp_dir().join(format!("wc-pm-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let root = dir.as_path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "user.name", "smoke"], root);
    git(&["config", "user.email", "smoke@local"], root);
    std::fs::write(root.join("src/lib.rs"), "// committed\n").unwrap();
    git(&["add", "src/lib.rs"], root);
    git(&["commit", "-q", "-m", "init"], root);

    let target =
        normalize_target("src/lib.rs", root).unwrap_or_else(|e| fail(&format!("normalize: {e}")));
    let plan = WritePlan {
        run_id: "smoke".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from("impl-0"),
        canonical_root: root.to_path_buf(),
        isolated_root: root.join(".archon/wc/smoke/impl-0"),
        target_files: vec![target],
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: BTreeSet::new(),
    };
    let cfg = WriteCoordinatorConfig::default();
    let baseline = capture_canonical_baseline(root, &plan, &[], &cfg)
        .unwrap_or_else(|e| fail(&format!("baseline: {e}")));
    let ws = create_item_workspace(root, &plan, &baseline)
        .unwrap_or_else(|e| fail(&format!("workspace: {e}")));
    // Agent edits the declared target inside the isolated workspace.
    std::fs::write(plan.isolated_root.join("src/lib.rs"), "// agent edit\n").unwrap();
    let captured = capture_patch(&ws, &plan.target_files, &baseline)
        .unwrap_or_else(|e| fail(&format!("capture: {e}")));
    if captured.changed_files != ["src/lib.rs"] {
        fail(&format!(
            "unexpected changed_files: {:?}",
            captured.changed_files
        ));
    }
    validate_patch(&captured, &plan, &cfg, "Implemented the change.")
        .unwrap_or_else(|e| fail(&format!("validate: {e}")));
    // An undeclared write must be rejected.
    let mut bad = captured.clone();
    bad.changed_files = vec!["src/evil.rs".into()];
    if validate_patch(&bad, &plan, &cfg, "ok").is_ok() {
        fail("undeclared write was NOT rejected");
    }
    let run_root = root.join(".archon/workflows/smoke");
    let manifest_path = persist_manifest(
        &run_root,
        "smoke",
        "impl",
        &plan.item_id,
        &captured,
        ManifestStatus::PendingApply,
    )
    .unwrap_or_else(|e| fail(&format!("persist: {e}")));
    if !manifest_path.exists() {
        fail("manifest not persisted");
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!(
        "patch_manifest smoke: captured 1 file, validated, undeclared-rejected, manifest at {}",
        manifest_path.display()
    );
}

/// TASK-WC-004 — schedule disjoint vs overlapping items into waves.
fn smoke_conflict_graph(repo_root: &Path) {
    use archon_workflow::write_coordinator::conflict_graph::{
        WaveCaps, build_schedule, schedule_summary,
    };
    use archon_workflow::write_coordinator::write_plan::{
        ResourceKey, TargetFilesSource, normalize_target,
    };
    use archon_workflow::write_coordinator::{ItemId, WritePlan};
    use std::collections::BTreeMap;

    let mk = |id: &str, key: ResourceKey| {
        let target = normalize_target("crates/archon-workflow/src/lib.rs", repo_root)
            .unwrap_or_else(|e| fail(&format!("normalize: {e}")));
        WritePlan {
            run_id: "smoke".into(),
            stage_id: "impl".into(),
            item_id: ItemId::from(id),
            canonical_root: repo_root.to_path_buf(),
            isolated_root: repo_root.join(".archon/wc").join(id),
            target_files: vec![target],
            target_files_source: TargetFilesSource::Item,
            read_context_files: vec![],
            verify_inputs: vec![],
            baseline_id: "git:HEAD".into(),
            workspace_boundary_required: true,
            resource_keys: [key].into_iter().collect(),
        }
    };

    let plans = vec![
        mk("a", ResourceKey::File("src/a.rs".into())),
        mk("b", ResourceKey::File("src/b.rs".into())),
        mk("c", ResourceKey::File("src/a.rs".into())),
    ];
    let caps = WaveCaps::from_sources(8, 4, None, None, None);
    let schedule = build_schedule("impl", &plans, &BTreeMap::new(), &caps).unwrap_or_else(|e| {
        fail(&format!("build_schedule: {e}"));
    });
    let summary = schedule_summary(&schedule);
    // a and b disjoint -> wave 0; c overlaps a -> wave 1.
    if schedule.waves.len() != 2 {
        fail(&format!("expected 2 waves, got {}", schedule.waves.len()));
    }
    if summary.max_width != 2 {
        fail(&format!("expected max_width 2, got {}", summary.max_width));
    }
    println!(
        "conflict_graph smoke: {} waves, max_width {}, widths {:?}",
        summary.wave_count,
        summary.max_width,
        schedule
            .waves
            .iter()
            .map(|w| w.items.len())
            .collect::<Vec<_>>()
    );
}

/// TASK-WC-003 — build a throwaway git repo, isolate an item, prove the dirty
/// overlay reproduces, mutation detection fires, and cleanup removes the tree.
fn smoke_worktree_isolation() {
    use archon_workflow::WriteCoordinatorConfig;
    use archon_workflow::write_coordinator::worktree_isolation::{
        WorkspaceStatus, capture_canonical_baseline, cleanup_workspace, create_item_workspace,
        detect_canonical_mutation,
    };
    use archon_workflow::write_coordinator::write_plan::{TargetFilesSource, normalize_target};
    use archon_workflow::write_coordinator::{ItemId, WritePlan};
    use std::collections::BTreeSet;

    fn git(args: &[&str], cwd: &Path) {
        let out = std::process::Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap_or_else(|e| fail(&format!("git spawn: {e}")));
        if !out.status.success() {
            fail(&format!(
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }

    let dir = std::env::temp_dir().join(format!("wc-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap_or_else(|e| fail(&format!("mkdir: {e}")));
    let root = dir.as_path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "user.name", "smoke"], root);
    git(&["config", "user.email", "smoke@local"], root);
    std::fs::write(root.join("src/lib.rs"), "// committed\n").unwrap();
    git(&["add", "src/lib.rs"], root);
    git(&["commit", "-q", "-m", "init"], root);
    // Dirty tracked overlay + an undeclared secret that must NOT leak.
    std::fs::write(root.join("src/lib.rs"), "// dirty edit\n").unwrap();
    std::fs::write(root.join(".env"), "SECRET=leakme\n").unwrap();

    let target = normalize_target("src/lib.rs", root)
        .unwrap_or_else(|e| fail(&format!("normalize target: {e}")));
    let plan = WritePlan {
        run_id: "smoke".into(),
        stage_id: "impl".into(),
        item_id: ItemId::from("impl-0"),
        canonical_root: root.to_path_buf(),
        isolated_root: root.join(".archon/wc/smoke/impl-0"),
        target_files: vec![target],
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "git:HEAD".into(),
        workspace_boundary_required: true,
        resource_keys: BTreeSet::new(),
    };
    let cfg = WriteCoordinatorConfig::default();
    let baseline = capture_canonical_baseline(root, &plan, &[], &cfg)
        .unwrap_or_else(|e| fail(&format!("capture: {e}")));
    if baseline.untracked_files.contains_key(".env") {
        fail("SECRET .env leaked into baseline");
    }
    let ws = create_item_workspace(root, &plan, &baseline)
        .unwrap_or_else(|e| fail(&format!("workspace: {e}")));
    let overlay = std::fs::read_to_string(plan.isolated_root.join("src/lib.rs")).unwrap();
    if overlay != "// dirty edit\n" {
        fail("dirty overlay did not reproduce in isolated worktree");
    }
    if plan.isolated_root.join(".env").exists() {
        fail("SECRET .env leaked into isolated worktree");
    }

    std::fs::write(root.join("src/lib.rs"), "// mutated behind back\n").unwrap();
    match detect_canonical_mutation(root, &baseline, &plan.target_files, &[]) {
        Err(_) => println!(
            "worktree smoke: mutation detected, baseline_commit={}",
            ws.baseline_commit
        ),
        Ok(()) => fail("canonical mutation was NOT detected"),
    }
    cleanup_workspace(root, &plan.isolated_root, WorkspaceStatus::Succeeded, &cfg)
        .unwrap_or_else(|e| fail(&format!("cleanup: {e}")));
    if plan.isolated_root.exists() {
        fail("cleanup did not remove isolated worktree");
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!("worktree smoke: isolation + overlay + secret-block + cleanup OK");
}

fn fail(msg: &str) -> ! {
    eprintln!("SMOKE FAILURE: {msg}");
    exit(1);
}
