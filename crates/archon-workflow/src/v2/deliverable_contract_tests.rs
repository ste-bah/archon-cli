//! Issue-22: a declared relative path resolves under the project artifact
//! root first, then the target repository root. Every test here drives the
//! REAL generated verifier through `sh` where the shell is involved, because
//! the live false negative was produced by that script, not by a Rust stub.

use super::{ContractRoots, resolve_contract_path, typed_verification_command};

struct Roots {
    project: tempfile::TempDir,
    repository: tempfile::TempDir,
}

impl Roots {
    fn new() -> Self {
        Self {
            project: tempfile::tempdir().expect("project"),
            repository: tempfile::tempdir().expect("repository"),
        }
    }

    fn contract_roots(&self) -> ContractRoots {
        ContractRoots::new(self.project_str(), Some(&self.repository_str()))
    }

    fn project_str(&self) -> String {
        self.project.path().to_string_lossy().replace('\\', "/")
    }

    fn repository_str(&self) -> String {
        self.repository.path().to_string_lossy().replace('\\', "/")
    }

    fn write(dir: &std::path::Path, relative: &str, bytes: &str) {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(path, bytes).expect("file");
    }
}

fn run_verifier(script: &str) -> (bool, String) {
    let mut child = std::process::Command::new(archon_shell::resolve_posix_shell())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn verifier");
    use std::io::Write as _;
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write verifier");
    let output = child.wait_with_output().expect("verifier output");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .replace("\\\\", "/"),
    )
}

#[test]
fn a_relative_path_present_only_under_the_repository_resolves_there() {
    let roots = Roots::new();
    Roots::write(
        roots.repository.path(),
        "crates/x/src/lib.rs",
        "pub fn x() {}",
    );
    let resolved = resolve_contract_path(
        &roots.contract_roots(),
        Some(&serde_json::json!("crates/x/src/lib.rs")),
    );
    assert_eq!(
        resolved,
        format!("{}/crates/x/src/lib.rs", roots.repository_str())
    );
}

#[test]
fn a_relative_path_present_under_both_roots_resolves_to_the_project() {
    let roots = Roots::new();
    Roots::write(roots.project.path(), "out/report.md", "project copy");
    Roots::write(roots.repository.path(), "out/report.md", "repository copy");
    let resolved = resolve_contract_path(
        &roots.contract_roots(),
        Some(&serde_json::json!("out/report.md")),
    );
    assert_eq!(resolved, format!("{}/out/report.md", roots.project_str()));
}

#[test]
fn an_absolute_path_ignores_every_root() {
    let roots = Roots::new();
    let resolved = resolve_contract_path(
        &roots.contract_roots(),
        Some(&serde_json::json!("/elsewhere/out.json")),
    );
    assert_eq!(resolved, "/elsewhere/out.json");
}

#[test]
fn a_single_root_resolves_exactly_as_before_whether_or_not_the_file_exists() {
    let roots = Roots::new();
    let single = ContractRoots::project_only(roots.project_str());
    let missing = resolve_contract_path(&single, Some(&serde_json::json!("out/none.json")));
    assert_eq!(missing, format!("{}/out/none.json", roots.project_str()));
    Roots::write(roots.project.path(), "out/some.json", "{}");
    let present = resolve_contract_path(&single, Some(&serde_json::json!("out/some.json")));
    assert_eq!(present, format!("{}/out/some.json", roots.project_str()));
    // Under no root with two roots known: reported against the project root.
    let missing = resolve_contract_path(
        &roots.contract_roots(),
        Some(&serde_json::json!("out/none.json")),
    );
    assert_eq!(missing, format!("{}/out/none.json", roots.project_str()));
}

#[test]
fn the_typed_verifier_is_handed_the_repository_path_when_the_artifact_lives_there() {
    let roots = Roots::new();
    Roots::write(
        roots.repository.path(),
        "crates/x/src/lib.rs",
        "pub fn x() {}",
    );
    Roots::write(roots.project.path(), ".archon/registry.json", "{}");
    let contract = serde_json::json!({
        "artifact_path": "crates/x/src/lib.rs",
        "registry_path": ".archon/registry.json",
        "typed_verifier_command": "check {artifact_path} {registry_path}",
    });
    let command =
        typed_verification_command(&roots.contract_roots(), &contract).expect("typed command");
    assert_eq!(
        command,
        format!(
            "check '{}/crates/x/src/lib.rs' '{}/.archon/registry.json'",
            roots.repository_str(),
            roots.project_str()
        )
    );
}

/// The live failure: a repository-relative source file that exists in the
/// repository was reported "missing or empty" because only the project root
/// was consulted. The generated verifier must find it under the second root.
#[test]
fn the_shell_verifier_finds_a_deliverable_that_exists_only_in_the_repository() {
    let roots = Roots::new();
    Roots::write(
        roots.repository.path(),
        "crates/x/src/lib.rs",
        "pub fn x() {}",
    );
    let contract = serde_json::json!({
        "kind": "source_module",
        "artifact_path": "crates/x/src/lib.rs",
    });
    let script = super::verification_command(&roots.contract_roots(), &contract);
    let (passed, output) = run_verifier(&script);
    assert!(passed, "{output}");
    assert!(
        output.contains("declared_text_deliverable_present"),
        "{output}"
    );
    assert!(
        output.contains(&format!("{}/crates/x/src/lib.rs", roots.repository_str())),
        "the verdict must carry the repository resolution: {output}"
    );
}

#[test]
fn the_shell_verifier_names_every_root_when_a_deliverable_is_under_none() {
    let roots = Roots::new();
    let contract = serde_json::json!({
        "kind": "source_module",
        "artifact_path": "crates/x/src/lib.rs",
    });
    let script = super::verification_command(&roots.contract_roots(), &contract);
    let (passed, output) = run_verifier(&script);
    assert!(!passed, "{output}");
    let expected = format!(
        "declared deliverable missing or empty: crates/x/src/lib.rs (looked under {}, {})",
        roots.project_str(),
        roots.repository_str()
    );
    assert!(output.contains(&expected), "{output}");
}

/// One root known: the failure text is the historical resolved path, so
/// nothing that parses today's verdicts sees a different shape.
#[test]
fn the_shell_verifier_keeps_the_single_root_failure_text() {
    let roots = Roots::new();
    let contract = serde_json::json!({"artifact_path": "crates/x/src/lib.rs"});
    let single = ContractRoots::project_only(roots.project_str());
    let (passed, output) = run_verifier(&super::verification_command(&single, &contract));
    assert!(!passed, "{output}");
    assert!(
        output.contains(&format!(
            "declared deliverable missing or empty: {}/crates/x/src/lib.rs",
            roots.project_str()
        )),
        "{output}"
    );
    assert!(!output.contains("looked under"), "{output}");
}

/// A JSON deliverable found under the repository is then PARSED from there:
/// the root that located the file is the one every later predicate uses.
#[test]
fn the_shell_verifier_reads_a_repository_json_deliverable_and_its_registry_from_there() {
    let roots = Roots::new();
    Roots::write(
        roots.repository.path(),
        "out/matrix.json",
        r#"{"cells": [{"name": "one"}]}"#,
    );
    Roots::write(
        roots.repository.path(),
        "out/registry.json",
        r#"{"records": {}}"#,
    );
    let contract = serde_json::json!({
        "artifact_path": "out/matrix.json",
        "registry_path": "out/registry.json",
    });
    let script = super::verification_command(&roots.contract_roots(), &contract);
    let (passed, output) = run_verifier(&script);
    assert!(passed, "{output}");
    assert!(output.contains("declared_deliverable_present"), "{output}");
}

/// Glob binding walks the same root order: instances under the repository
/// are found when the project holds none.
#[test]
fn the_shell_verifier_globs_instances_under_the_repository_root() {
    let roots = Roots::new();
    Roots::write(roots.repository.path(), "runs/a/report.md", "a");
    Roots::write(roots.repository.path(), "runs/b/report.md", "b");
    let contract = serde_json::json!({
        "artifact_path": "runs/<run-id>/report.md",
        "min_instances": 2,
    });
    let script = super::verification_command(&roots.contract_roots(), &contract);
    let (passed, output) = run_verifier(&script);
    assert!(passed, "{output}");
    assert!(output.contains(r#""instance_count": 2"#), "{output}");
}
