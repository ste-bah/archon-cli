use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;

fn runs(stderr: &str) -> BaselineRuns {
    let result = archon_workflow::acceptance_scratch::CheckResult {
        fault: None,
        acceptance_id: "AC-X-001".into(),
        exit_code: Some(1),
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
        environment_note: None,
        operational_error: None,
        classification: None,
    };
    BaselineRuns {
        reuse: BTreeMap::new(),
        commit: "tree".into(),
        failures: BTreeMap::from([(
            "AC-X-001".into(),
            super::super::super::executability::CheckEvidence {
                result,
                copy_layout: None,
            },
        )]),
        environment: BTreeMap::new(),
        forwarded: Vec::new(),
    }
}

#[cfg(unix)]
fn executable(dir: &Path, name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join(name);
    std::fs::write(&path, "#!/bin/sh\nexit 2\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn wrapped_parser_refusals_are_extracted_from_real_external_output() {
    let repository = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let path = executable(external.path(), "neutral-x");
    let output = "X FAIL: lanes failed: error: unexpected argument 'name_b' found\n\nUsage: neutral-x test [OPTIONS] [TESTNAME] [-- [ARGS...]]\n\nFor more information, try '--help'.\n";
    let actual = super::external_usage_error_on_path(
        &runs(output),
        "AC-X-001",
        repository.path(),
        repository.path(),
        "neutral-x test name_a name_b",
        external.path().as_os_str(),
    );
    assert_eq!(
        actual,
        Some((
            "neutral-x".into(),
            path,
            "error: unexpected argument 'name_b' found".into(),
        )),
        "the wrapper prefix must not hide the parser refusal"
    );

    let clap = "error: unexpected argument '--exact' found\n\n  tip: to pass '--exact' as a value, use '-- --exact'\n\nUsage: neutral-x test [OPTIONS] [TESTNAME] [-- [ARGS...]]\n\nFor more information, try '--help'.\n";
    assert!(
        super::external_usage_error_on_path(
            &runs(clap),
            "AC-X-001",
            repository.path(),
            repository.path(),
            "neutral-x test --exact",
            external.path().as_os_str(),
        )
        .is_some()
    );
}

#[cfg(unix)]
#[test]
fn wrapped_refusals_keep_repository_binaries_and_product_prose_on_the_judge_path() {
    let repository = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    executable(external.path(), "neutral-x");
    std::fs::write(
        repository.path().join("Cargo.toml"),
        "[package]\nname = \"fixture-core\"\nversion = \"0.1.0\"\n[[bin]]\nname = \"neutral-x\"\npath = \"src/main.rs\"\n",
    )
    .unwrap();
    let wrapped = "X FAIL: lanes failed: error: unexpected argument 'name_b' found\n\nUsage: neutral-x test [OPTIONS]\n";
    assert_eq!(
        super::external_usage_error_on_path(
            &runs(wrapped),
            "AC-X-001",
            repository.path(),
            repository.path(),
            "neutral-x test name_a name_b",
            external.path().as_os_str(),
        ),
        None
    );
    assert_eq!(
        super::external_usage_error_on_path(
            &runs("step 2: error: unexpected argument handling is not implemented\n"),
            "AC-X-001",
            repository.path(),
            repository.path(),
            "neutral-x test",
            external.path().as_os_str(),
        ),
        None
    );
}

#[cfg(unix)]
#[test]
fn findings_show_the_path_entry_without_canonicalizing_it() {
    use std::os::unix::fs::symlink;

    let repository = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let real = executable(external.path(), "neutral-x-real");
    let link = external.path().join("neutral-x");
    symlink(&real, &link).unwrap();
    let actual = super::external_usage_error_on_path(
        &runs("error: unexpected argument '--x' found\nUsage: neutral-x [OPTIONS]\n"),
        "AC-X-001",
        repository.path(),
        repository.path(),
        "neutral-x --x",
        external.path().as_os_str(),
    );
    assert_eq!(
        actual,
        Some((
            "neutral-x".into(),
            link,
            "error: unexpected argument '--x' found".into(),
        ))
    );
}
