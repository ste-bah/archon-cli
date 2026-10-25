#![cfg(target_os = "macos")]

use archon_workflow::acceptance_scratch::faults::FaultCause;
use archon_workflow::acceptance_scratch::{ScratchPolicy, observe_commands};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::{AcceptanceContract, content_digest};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[tokio::test]
async fn attributed_teardown_is_not_reported_again_during_final_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let _clear = ClearImmutable(temp.path().join("scratch"));
    let repository = temp.path().join("repository");
    let project = temp.path().join("project");
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::create_dir_all(project.join("data")).unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    git(&repository, &["init", "-q"]);
    git(
        &repository,
        &["config", "user.email", "test@example.invalid"],
    );
    git(&repository, &["config", "user.name", "test"]);
    std::fs::write(repository.join("input"), "source").unwrap();
    git(&repository, &["add", "."]);
    git(&repository, &["commit", "-qm", "fixture"]);
    let commit = git(&repository, &["rev-parse", "HEAD"]);
    std::fs::write(project.join("data/value"), "before").unwrap();

    let commands = [
        (
            "printf changed > input && mkdir blocked && touch blocked/keep && chflags uchg blocked/keep",
            "repo_root",
        ),
        ("test -f data/value", "project_root"),
    ];
    let acceptance = commands
        .iter()
        .enumerate()
        .map(|(index, (command, _))| {
            serde_json::json!({
                "id": format!("AC-X-00{}", index + 1),
                "criterion": "output correct",
                "check": {"kind": "command", "command": command, "cwd": commands[index].1},
                "judgment": {
                    "verdict": "accepted",
                    "counterexample": "incorrect output",
                    "reason": "checks output",
                    "host_call_id": "j"
                }
            })
        })
        .collect::<Vec<_>>();
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "p", "digest": "d"},
        "gap_policy": {
            "permitted_acceptance_ids": [],
            "forbidden_phrases": [],
            "required_fields": []
        },
        "acceptance": acceptance
    }))
    .unwrap();
    let refs = commands
        .iter()
        .enumerate()
        .map(|(index, (command, _))| FrozenCommandRef {
            acceptance_id: format!("AC-X-00{}", index + 1),
            kind: AcceptanceCommandKind::Command,
            chain_digest: "chain".into(),
            command_digest: content_digest(command.as_bytes()),
        })
        .collect::<Vec<_>>();
    let policy = ScratchPolicy {
        repository,
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: temp.path().join("scratch"),
        project_inputs: vec!["data".into()],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: "/usr/bin:/bin:/usr/sbin:/sbin".into(),
        environment: BTreeMap::new(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 20,
        output_bytes: 2048,
        scratch_bytes: 16 * 1024 * 1024,
        build_cache: None,
    };

    let result = observe_commands(
        &policy,
        &commit,
        &contract,
        "chain",
        &refs,
        &temp.path().join("evidence"),
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
    .unwrap();

    assert_eq!(result.checks.len(), 2, "{result:?}");
    assert!(
        matches!(
            result.checks[0].fault,
            Some(FaultCause::IdentityChanged { .. })
        ),
        "{result:?}"
    );
    let attribution_notes = result
        .checks
        .iter()
        .filter_map(|check| check.operational_error.as_deref())
        .map(|message| {
            message
                .matches("the scratch site could not be removed after this check:")
                .count()
        })
        .sum::<usize>();
    assert_eq!(attribution_notes, 1, "{result:?}");
    // D-A: this note moved from operational_errors to the typed attribution
    // field so an attributed check does not void its batch.
    assert!(result.operational_errors.is_empty(), "{result:?}");
    assert!(
        result.attributed_teardown.as_ref().is_some_and(|teardown| {
            teardown.acceptance_id == "AC-X-001"
                && teardown
                    .error
                    .contains("scratch site contains an unremovable path")
        }),
        "{result:?}"
    );
    assert!(result.teardown_verified, "{result:?}");
    assert!(
        result.checks[1]
            .operational_error
            .as_deref()
            .is_some_and(|error| error.contains("host-unproven")),
        "{result:?}"
    );
}

struct ClearImmutable(PathBuf);

impl Drop for ClearImmutable {
    fn drop(&mut self) {
        let _ = Command::new("chflags")
            .args(["-R", "nouchg"])
            .arg(&self.0)
            .status();
    }
}
