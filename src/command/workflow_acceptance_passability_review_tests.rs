use super::*;

#[test]
#[allow(non_snake_case)]
fn R3_static_package_selector_uses_manifest_names_before_any_execution() {
    let repo = repository();
    std::fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname='existing'\nversion='0.1.0'\n[workspace]\nmembers=[]\n",
    )
    .unwrap();
    assert!(setup_git(repo.path(), &["add", "."]).is_some());
    assert!(setup_git(repo.path(), &["commit", "-qm", "workspace"]).is_some());
    let (mut contract, _) = case("", vec![]);
    for action in ["test", "build"] {
        contract.acceptance[0].check = serde_json::from_value(serde_json::json!({"kind":"command", "cwd":"repo_root", "command":format!("set -e; cargo test --test new_target; cargo {action} -p {}", repo.path().file_name().unwrap().to_string_lossy())})).unwrap();
        let findings = super::super::super::setup::package_findings(
            &contract,
            repo.path(),
            repo.path(),
            "HEAD",
            false,
        );
        assert!(
            findings["check"].contains("names directory")
                && findings["check"].contains("its package is named existing")
        );
        contract.acceptance[0].check = serde_json::from_value(serde_json::json!({"kind":"command", "cwd":"repo_root", "command":format!("cargo {action} -p existing")})).unwrap();
        assert!(
            super::super::super::setup::package_findings(
                &contract,
                repo.path(),
                repo.path(),
                "HEAD",
                false,
            )
            .is_empty()
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn R3_missing_new_test_targets_are_judged_with_absence_evidence() {
    let repo = repository();
    // Behavioral requirements need not predict the names of tests the work adds.
    for target in [
        "adversarial_review",
        "behavior_probe",
        "migration_preserves_artifacts",
    ] {
        let (mut contract, mut candidate) = case(
            &format!("no test target named `{target}`"),
            vec!["R".into()],
        );
        contract.acceptance[0].check = serde_json::from_value(serde_json::json!({"kind":"command", "cwd":"repo_root", "command":format!("cargo test -p existing --test {target}")})).unwrap();
        let requirements = [("R".into(), "Adding coverage for preserved behavior".into())].into();
        assert!(
            diagnose(
                &mut candidate,
                &contract,
                &requirements,
                repo.path(),
                repo.path(),
                "HEAD",
            )
            .is_none()
        );
        assert!(
            candidate.shown["base_setup"]["fact"]
                .as_str()
                .unwrap()
                .contains("absent at base")
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn R5_D9_setup_requires_whole_name_and_add_words() {
    let repo = repository();
    let (contract, mut candidate) = case(
        "package ID specification `missing` did not match any packages",
        vec!["R".into()],
    );
    let requirements = [("R".into(), "Address missing metadata".into())].into();
    assert!(
        diagnose(
            &mut candidate,
            &contract,
            &requirements,
            repo.path(),
            repo.path(),
            "HEAD"
        )
        .is_some()
    );
}

fn package_case(name: &str) -> BTreeMap<String, String> {
    package_case_at(name, None)
}

fn package_case_at(name: &str, cd_operator: Option<&str>) -> BTreeMap<String, String> {
    let parent = tempfile::tempdir().unwrap();
    let repo = parent.path().join("archon-cli");
    std::fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "user.name", "test"],
    ] {
        assert!(setup_git(&repo, &args).is_some());
    }
    std::fs::write(repo.join("Cargo.toml"), "[package]\nname='archon-cli-workspace'\nversion='0.1.0'\n[workspace]\nmembers=['member-dir']\n").unwrap();
    std::fs::create_dir(repo.join("member-dir")).unwrap();
    std::fs::write(
        repo.join("member-dir/Cargo.toml"),
        "[package]\nname='real-package'\nversion='0.1.0'\n",
    )
    .unwrap();
    assert!(setup_git(&repo, &["add", "."]).is_some());
    assert!(setup_git(&repo, &["commit", "-qm", "workspace"]).is_some());
    let project = parent.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let (mut contract, _) = case("", vec![]);
    let mut all = BTreeMap::new();
    for action in ["test", "build"] {
        let (cwd, command) = match cd_operator {
            Some(op) => (
                "project_root",
                format!("cd ../archon-cli {op} cargo {action} -p {name}"),
            ),
            None => ("repo_root", format!("cargo {action} -p {name}")),
        };
        contract.acceptance[0].check = serde_json::from_value(
            serde_json::json!({"kind":"command", "cwd":cwd, "command":command}),
        )
        .unwrap();
        all.extend(super::super::super::setup::package_findings(
            &contract, &repo, &project, "HEAD", false,
        ));
    }
    all
}
#[test]
#[allow(non_snake_case)]
fn B2_root_directory_selector_names_its_real_package() {
    // Input: root directory name instead of package -> deterministic finding.
    let findings = package_case("archon-cli");
    assert!(findings["check"].contains(
        "`-p archon-cli` names directory archon-cli; its package is named archon-cli-workspace"
    ));
}
#[test]
#[allow(non_snake_case)]
fn B2_new_crate_selector_is_left_to_base_and_judge() {
    // Input: unknown package the work may add -> no static finding.
    assert!(package_case("new-crate").is_empty());
}
#[test]
#[allow(non_snake_case)]
fn B2_real_package_selector_has_no_static_finding() {
    // Input: actual workspace package name -> no static finding.
    assert!(package_case("real-package").is_empty());
}
#[test]
#[allow(non_snake_case)]
fn B2_member_directory_selector_names_its_real_package() {
    // Input: member directory name instead of package -> deterministic finding.
    assert!(package_case("member-dir")["check"].contains("its package is named real-package"));
}

#[test]
#[allow(non_snake_case)]
fn S6_project_sibling_cd_resolves_the_effective_cargo_workspace() {
    // Input: project beside repository, cd by && or ; before test/build.
    // Expected: directory/package confusion is named; a new crate is not refused.
    for op in ["&&", ";"] {
        let findings = package_case_at("archon-cli", Some(op));
        assert!(findings["check"].contains("its package is named archon-cli-workspace"));
        assert!(package_case_at("new-crate", Some(op)).is_empty());
        assert!(package_case_at("real-package", Some(op)).is_empty());
    }
}

#[path = "workflow_acceptance_passability_packages_tests.rs"]
mod packages_tests;
