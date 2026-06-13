//! TASK-WC-001 — Config and spec wiring for the Write Coordinator.
//!
//! Covers: config defaults + round-trip, runtime resolution (git vs non-git
//! vs feature-disabled), spec-level per-item target validation, and the
//! canonical ItemId/WaveId alias exports.

use std::fs;

use archon_workflow::spec::WorkflowSpec;
use archon_workflow::write_coordinator::{
    SerialFallbackReason, WriteCoordinatorConfig, WriteCoordinatorRuntime,
    resolve_write_coordinator_runtime,
};
use archon_workflow::{WorkflowConfig, WorkflowError, WorkflowPolicy};

fn impl_fanout_yaml(items_yaml: &str) -> String {
    format!(
        r#"
schema: archon.workflow.v1
name: wc-001-test
task: validate write coordination
stages:
  - id: impl
    kind: fanout
    item_kind: implementation
    input:
      items:
{items_yaml}
"#
    )
}

#[test]
fn config_defaults_when_block_absent() {
    let cfg: WorkflowConfig = toml::from_str("").expect("empty workflow config deserializes");
    let wc = cfg.write_coordinator;
    assert!(wc.enabled);
    assert!(!wc.retain_success_worktrees);
    assert!(wc.retain_failed_worktrees);
    assert_eq!(wc.max_patch_bytes, 10_485_760);
    assert_eq!(wc.max_file_bytes, 1_048_576);
    assert!(wc.fail_on_undeclared_write);
    assert!(wc.allow_dirty_canonical_repo);
}

#[test]
fn config_enabled_false_round_trips() {
    let toml_src = "[write_coordinator]\nenabled = false\n";
    let cfg: WorkflowConfig = toml::from_str(toml_src).expect("deserializes");
    assert!(!cfg.write_coordinator.enabled);
    let serialized = toml::to_string(&cfg).expect("serializes");
    let reparsed: WorkflowConfig = toml::from_str(&serialized).expect("round-trips");
    assert_eq!(cfg, reparsed);
    assert!(!reparsed.write_coordinator.enabled);
}

#[test]
fn workflow_policy_carries_write_coordinator_config() {
    let cfg: WorkflowConfig =
        toml::from_str("[write_coordinator]\nenabled = false\nmax_patch_bytes = 1024\n")
            .expect("deserializes");
    let policy = WorkflowPolicy::from_config(&cfg);
    assert!(!policy.write_coordinator.enabled);
    assert_eq!(policy.write_coordinator.max_patch_bytes, 1024);
}

#[test]
fn resolve_returns_disabled_non_git_root_for_plain_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = WriteCoordinatorConfig::default();
    let runtime = resolve_write_coordinator_runtime(dir.path(), &cfg);
    assert!(matches!(
        runtime,
        WriteCoordinatorRuntime::Disabled {
            reason: SerialFallbackReason::NonGitRoot
        }
    ));
}

#[test]
fn resolve_returns_enabled_for_git_dir_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join(".git")).expect("mkdir .git");
    let cfg = WriteCoordinatorConfig::default();
    match resolve_write_coordinator_runtime(dir.path(), &cfg) {
        WriteCoordinatorRuntime::Enabled { canonical_root } => {
            assert_eq!(canonical_root, dir.path().to_path_buf());
        }
        other => panic!("expected Enabled, got {other:?}"),
    }
}

#[test]
fn resolve_returns_enabled_for_git_file_root() {
    // Linked worktrees have a `.git` FILE pointing at the real gitdir.
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join(".git"),
        "gitdir: /elsewhere/.git/worktrees/x\n",
    )
    .expect("write");
    let cfg = WriteCoordinatorConfig::default();
    assert!(matches!(
        resolve_write_coordinator_runtime(dir.path(), &cfg),
        WriteCoordinatorRuntime::Enabled { .. }
    ));
}

#[test]
fn resolve_returns_feature_disabled_even_with_git_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join(".git")).expect("mkdir .git");
    let cfg = WriteCoordinatorConfig {
        enabled: false,
        ..WriteCoordinatorConfig::default()
    };
    assert!(matches!(
        resolve_write_coordinator_runtime(dir.path(), &cfg),
        WriteCoordinatorRuntime::Disabled {
            reason: SerialFallbackReason::FeatureDisabled
        }
    ));
}

#[test]
fn validate_errs_when_items_declare_no_targets_and_fail_on_undeclared_write() {
    let yaml = impl_fanout_yaml("        - name: alpha\n        - name: beta");
    let spec = WorkflowSpec::from_yaml(&yaml).expect("base spec valid");
    let cfg = WriteCoordinatorConfig::default();
    let err = spec
        .validate_write_coordination(&cfg)
        .expect_err("must reject undeclared per-item targets");
    match err {
        WorkflowError::ImplementationFanoutMissingPerItemTargets { stage, item } => {
            assert_eq!(stage, "impl");
            assert_eq!(item, "impl-0");
        }
        other => panic!("wrong error variant: {other:?}"),
    }
}

#[test]
fn validate_ok_when_fail_on_undeclared_write_disabled() {
    let yaml = impl_fanout_yaml("        - name: alpha\n        - name: beta");
    let spec = WorkflowSpec::from_yaml(&yaml).expect("base spec valid");
    let cfg = WriteCoordinatorConfig {
        fail_on_undeclared_write: false,
        ..WriteCoordinatorConfig::default()
    };
    spec.validate_write_coordination(&cfg)
        .expect("degrades to conflicting items, not an error");
}

#[test]
fn validate_ok_when_items_declare_target_files() {
    let yaml = impl_fanout_yaml(
        "        - target_files: [\"src/a.rs\"]\n        - target_files: [\"src/b.rs\"]",
    );
    let spec = WorkflowSpec::from_yaml(&yaml).expect("base spec valid");
    let cfg = WriteCoordinatorConfig::default();
    spec.validate_write_coordination(&cfg)
        .expect("declared targets pass");
}

#[test]
fn validate_ok_when_items_declare_expected_target_files() {
    let yaml = impl_fanout_yaml(
        "        - expected_target_files: [\"src/a.rs\"]\n        - expected_target_files: [\"src/b.rs\"]",
    );
    let spec = WorkflowSpec::from_yaml(&yaml).expect("base spec valid");
    let cfg = WriteCoordinatorConfig::default();
    spec.validate_write_coordination(&cfg)
        .expect("expected_target_files alias passes");
}

#[test]
fn validate_skips_non_implementation_fanout() {
    let yaml = r#"
schema: archon.workflow.v1
name: wc-001-non-impl
task: validate non-implementation fanout untouched
stages:
  - id: fan
    kind: fanout
    input:
      items:
        - name: alpha
        - name: beta
"#;
    let spec = WorkflowSpec::from_yaml(yaml).expect("base spec valid");
    let cfg = WriteCoordinatorConfig::default();
    spec.validate_write_coordination(&cfg)
        .expect("non-implementation fanout is not checked");
}

#[test]
fn validate_skips_everything_when_coordinator_disabled() {
    let yaml = impl_fanout_yaml("        - name: alpha");
    let spec = WorkflowSpec::from_yaml(&yaml).expect("base spec valid");
    let cfg = WriteCoordinatorConfig {
        enabled: false,
        ..WriteCoordinatorConfig::default()
    };
    spec.validate_write_coordination(&cfg)
        .expect("disabled coordinator validates nothing");
}

#[test]
fn item_and_wave_id_aliases_exported_from_crate_root() {
    let item: archon_workflow::ItemId = "impl-0".to_string();
    let wave: archon_workflow::WaveId = 0;
    assert_eq!(item, "impl-0");
    assert_eq!(wave, 0u32);
}
