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
        } => println!("runtime resolved: Disabled(NonGitRoot) for {}", non_git.display()),
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

    println!("write_coordinator smoke: OK");
}

fn fail(msg: &str) -> ! {
    eprintln!("SMOKE FAILURE: {msg}");
    exit(1);
}
