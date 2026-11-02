use super::*;

fn isolated(test: &str) -> bool {
    if std::env::var_os("ARCHON_R4_BUNDLE_PROBE").is_some() {
        return true;
    }
    let home = tempfile::tempdir().unwrap();
    let output = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .args([test, "--nocapture"])
        .env("ARCHON_R4_BUNDLE_PROBE", "1")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    false
}

fn fixture() -> (tempfile::TempDir, WorkflowStore, WorkflowRun) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let spec = WorkflowSpec::from_yaml("schema: archon.workflow.v1\nname: host-grants\ntask: inspect\nstages:\n  - id: inspect\n    kind: agent\n    task: inspect\n").unwrap();
    let run = store.create_run(spec).unwrap();
    WorkflowBundle::create_for_run_with_operator_grants(
        &store,
        &run,
        "export default async function workflow(w) {}",
        WorkflowBundleOrigin::GeneratedHarness,
        &["DECLARED_API_KEY".into()],
    )
    .unwrap();
    (temp, store, run)
}

#[test]
fn operator_env_coordinated_project_tamper_adds_no_resume_grant() {
    if !isolated("operator_env_coordinated_project_tamper_adds_no_resume_grant") {
        return;
    }
    let (temp, store, run) = fixture();
    let compiled = store.run_dir(&run.id).join(COMPILED_SPEC_FILE);
    let bytes = std::fs::read_to_string(&compiled)
        .unwrap()
        .replace("DECLARED_API_KEY", "ATTACKER_API_KEY");
    std::fs::write(&compiled, &bytes).unwrap();
    let mut manifest = super::super::read_manifest(&store, &run.id).unwrap();
    manifest.compiled_hash = content_hash(bytes.as_bytes());
    std::fs::write(
        store.run_dir(&run.id).join(MANIFEST_FILE),
        toml::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    crate::approval::WorkflowApprovalStore::for_workflow_store(&store)
        .approve_run_once(temp.path(), &store, &run, "operator")
        .unwrap();
    WorkflowBundle::verify(&store, &run.id).unwrap();
    assert_eq!(
        WorkflowBundle::verified_operator_env_grants(&store, &run.id).unwrap(),
        expected_operator_keys()
    );
}

#[test]
fn operator_env_missing_host_record_grants_nothing() {
    if !isolated("operator_env_missing_host_record_grants_nothing") {
        return;
    }
    let (_temp, store, run) = fixture();
    let path = archon_shell::credential_paths::home()
        .join(".archon/operator-env-grants")
        .join(format!("{}.json", run.id));
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
    assert!(
        WorkflowBundle::verified_operator_env_grants(&store, &run.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn operator_env_launch_authority_cannot_be_overwritten() {
    if !isolated("operator_env_launch_authority_cannot_be_overwritten") {
        return;
    }
    let (_temp, store, run) = fixture();
    let rewrite = WorkflowBundle::create_for_run_with_operator_grants(
        &store,
        &run,
        "export default async function workflow(w) {}",
        WorkflowBundleOrigin::GeneratedHarness,
        &["ATTACKER_API_KEY".into()],
    );
    assert_eq!(
        rewrite.is_err(),
        archon_shell::read_boundary::operator_grants_supported()
    );
    assert_eq!(
        WorkflowBundle::verified_operator_env_grants(&store, &run.id).unwrap(),
        expected_operator_keys()
    );
}

#[test]
fn operator_env_r5_donor_identity_and_coordinated_local_tampering_grants_nothing() {
    if !isolated("operator_env_r5_donor_identity_and_coordinated_local_tampering_grants_nothing") {
        return;
    }
    let (_donor_temp, donor_store, donor) = fixture();
    let (_victim_temp, victim_store, victim) = fixture();
    // Copy every local authority-looking file and substitute the donor ID in state.
    std::fs::create_dir_all(victim_store.run_dir(&donor.id)).unwrap();
    for entry in std::fs::read_dir(donor_store.run_dir(&donor.id))
        .unwrap()
        .flatten()
    {
        if entry.path().is_file() {
            std::fs::copy(
                entry.path(),
                victim_store.run_dir(&victim.id).join(entry.file_name()),
            )
            .unwrap();
            std::fs::copy(
                entry.path(),
                victim_store.run_dir(&donor.id).join(entry.file_name()),
            )
            .unwrap();
        }
    }
    // Agent-controlled approvals and matching bundle hashes still confer no host authority.
    crate::approval::WorkflowApprovalStore::for_workflow_store(&victim_store)
        .approve_run_once(
            _victim_temp.path(),
            &victim_store,
            &donor,
            "forged-local-operator",
        )
        .unwrap();
    WorkflowBundle::verify(&victim_store, &donor.id).unwrap();
    assert!(
        WorkflowBundle::verified_operator_env_grants(&victim_store, &donor.id)
            .unwrap()
            .is_empty(),
        "a donor record must not authorize a different project/run directory"
    );
    let error = victim_store
        .load_state(&victim.id)
        .expect_err("substituted state ID must refuse resume");
    assert!(error.to_string().contains("run id mismatch"));
}

#[test]
fn operator_env_r5_uncontained_host_disables_forged_grants() {
    if !isolated("operator_env_r5_uncontained_host_disables_forged_grants") {
        return;
    }
    if matches!(
        archon_shell::write_boundary::mechanism(),
        Ok(archon_shell::write_boundary::Mechanism::SandboxExec
            | archon_shell::write_boundary::Mechanism::Landlock { .. })
    ) {
        return;
    }
    let (_temp, store, run) = fixture();
    std::fs::create_dir_all(archon_shell::credential_paths::operator_grant_root()).unwrap();
    std::fs::write(
        operator_grant_path(&run.id).unwrap(),
        b"[\"FORGED_API_KEY\"]",
    )
    .unwrap();
    assert!(
        WorkflowBundle::verified_operator_env_grants(&store, &run.id)
            .unwrap()
            .is_empty(),
        "files cannot enable grants without kernel containment"
    );
}

fn expected_operator_keys() -> Vec<&'static str> {
    if archon_shell::read_boundary::operator_grants_supported() {
        vec!["DECLARED_API_KEY"]
    } else {
        Vec::new()
    }
}

#[cfg(unix)]
#[test]
fn operator_env_r6_no_grant_launch_with_unwritable_grant_directory() {
    if !isolated("operator_env_r6_no_grant_launch_with_unwritable_grant_directory") {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let root = archon_shell::credential_paths::operator_grant_root();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o500)).unwrap();
    assert!(std::fs::write(root.join("write-probe"), b"probe").is_err());
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    for origin in [
        WorkflowBundleOrigin::GeneratedHarness,
        WorkflowBundleOrigin::SavedCommand,
    ] {
        let spec = WorkflowSpec::from_yaml("schema: archon.workflow.v1\nname: no-grants\ntask: inspect\nstages:\n  - id: inspect\n    kind: agent\n    task: inspect\n").unwrap();
        let run = store.create_run(spec).unwrap();
        WorkflowBundle::create_for_run_with_operator_grants(
            &store,
            &run,
            "export default async function workflow(w) {}",
            origin,
            &[],
        )
        .expect("a no-grant launch must not need writable host storage");
        WorkflowBundle::verify(&store, &run.id).unwrap();
        assert!(!operator_grant_path(&run.id).unwrap().exists());
        assert!(
            WorkflowBundle::verified_operator_env_grants(&store, &run.id)
                .unwrap()
                .is_empty()
        );
    }
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn operator_env_r6_granted_launch_persists_record() {
    if !isolated("operator_env_r6_granted_launch_persists_record") {
        return;
    }
    let (_temp, store, run) = fixture();
    let path = operator_grant_path(&run.id).unwrap();
    if archon_shell::read_boundary::operator_grants_supported() {
        let record: OperatorGrantRecord =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(record.run_id, run.id);
        assert_eq!(record.grants, ["DECLARED_API_KEY"]);
        assert_eq!(
            (record.project_root, record.run_dir),
            canonical_grant_binding(&store, &run.id).unwrap()
        );
    } else {
        assert!(
            !path.exists(),
            "withheld grants must not create an empty host record"
        );
    }
}
