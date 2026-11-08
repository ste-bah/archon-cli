use archon_workflow::v2::{
    deliverable_contract::{self, ContractRoots},
    lifecycle_policy::verify_options::prepare_verification_items,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn cli(root: &Path, contract: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_archon"))
        .args(["workflow", "verify-deliverable", "--root"])
        .arg(root)
        .args(["--contract-json", contract])
        .output()
        .unwrap()
}
fn verdict(output: &Output, code: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value["status"].is_string());
    if code == 1 {
        assert_eq!(
            value["failure_count"],
            value["failures"].as_array().unwrap().len()
        );
    }
    value
}

#[test]
fn passing_json_text_and_directory_have_verdicts() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("out.json"), b"{\r\n}\r\n").unwrap();
    fs::write(root.path().join("out.txt"), "evidence").unwrap();
    fs::create_dir(root.path().join("directory")).unwrap();
    fs::write(root.path().join("directory/evidence"), "x").unwrap();
    for (path, status) in [
        ("out.json", "declared_deliverable_present"),
        ("out.txt", "declared_text_deliverable_present"),
        ("directory", "declared_text_deliverable_present"),
    ] {
        let output = cli(
            root.path(),
            &json!({"kind":"evidence","artifact_path":path}).to_string(),
        );
        assert_eq!(verdict(&output, 0)["status"], status);
    }
}

#[test]
fn failed_missing_invalid_artifact_and_unknown_contract_field_have_findings() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("bad.json"), "bad").unwrap();
    for contract in [
        json!({"kind":"evidence","artifact_path":"missing.json"}),
        json!({"kind":"evidence","artifact_path":"bad.json"}),
        json!({"kind":"evidence","artifact_path":"bad.json","unknown":true}),
    ] {
        let output = cli(root.path(), &contract.to_string());
        assert_eq!(verdict(&output, 1)["status"], "failed");
    }
}

#[test]
fn malformed_empty_and_truncated_input_exit_two_on_stderr() {
    let root = tempfile::tempdir().unwrap();
    for text in ["invalid", "", "{\"kind\":"] {
        let output = cli(root.path(), text);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("contract input is not valid JSON")
        );
    }
}

fn shell(command: &str) -> Output {
    let mut child = Command::new(archon_shell::resolve_posix_shell())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(command.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
fn initialize() {
    deliverable_contract::set_verifier_program(Ok(env!("CARGO_BIN_EXE_archon").into()));
}

#[test]
fn generated_command_preserves_quotes_spaces_and_absolute_contract_paths() {
    initialize();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root with spaces and 'quote");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("a'b.json"), "{}").unwrap();
    let roots = ContractRoots::project_only(root.to_string_lossy());
    let command = deliverable_contract::verification_command(
        &roots,
        &json!({"kind":"evidence","artifact_path":"a'b.json"}),
    );
    verdict(&shell(&command), 0);
    let command = deliverable_contract::verification_command(
        &roots,
        &json!({"kind":"evidence","artifact_path":"/x/y/nonexistent-evidence.json"}),
    );
    let value = verdict(&shell(&command), 1);
    assert!(
        value["failures"][0]
            .as_str()
            .unwrap()
            .contains("/x/y/nonexistent-evidence.json")
    );
}

#[test]
fn exact_focused_verification_strings_run_for_single_bound_and_typed_contracts() {
    initialize();
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("out/a")).unwrap();
    fs::write(root.path().join("out/a/artifact.json"), "{}").unwrap();
    for (path, typed) in [
        ("out/a/artifact.json", false),
        ("out/<id>/artifact.json", false),
        ("out/a/artifact.json", true),
    ] {
        let mut contract = json!({"kind":"evidence","artifact_path":path,"min_instances":1});
        if typed {
            contract["typed_verifier_command"] = json!("test -s {artifact_path}");
        }
        let universe =
            json!({"tasks":[{"canonical_task_id":"T-1","deliverable_contracts":[contract]}]});
        let items = prepare_verification_items(
            vec![
                json!({"item_id":"verify","canonical_task_ids":["T-1"],"artifact_requirements":["out/a/artifact.json"]}),
            ],
            Some(root.path().to_str().unwrap()),
            &[],
            &universe,
        );
        let added = items
            .iter()
            .find(|item| item.get("deliverable_contract").is_some())
            .unwrap();
        verdict(&shell(added["focused_verification"].as_str().unwrap()), 0);
        let bound = &items[0]["focused_verification"];
        for command in bound.as_array().unwrap() {
            verdict(&shell(command.as_str().unwrap()), 0);
        }
    }
}

fn snapshot(path: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    fn visit(root: &Path, path: &Path, files: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.push((path.strip_prefix(root).unwrap().to_owned(), Vec::new()));
                visit(root, &path, files);
            } else {
                files.push((
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }
    visit(path, path, &mut files);
    files.sort();
    files
}

struct RestorePermissions(Vec<(std::path::PathBuf, fs::Permissions)>);
impl Drop for RestorePermissions {
    fn drop(&mut self) {
        for (path, permissions) in &self.0 {
            let _ = fs::set_permissions(path, permissions.clone());
        }
    }
}

#[test]
fn evaluation_precedes_bootstrap_and_creates_no_files() {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().join("empty-home");
    let root = temporary.path().join("read-only-root");
    let cwd = temporary.path().join("cwd");
    for path in [&home, &root, &cwd] {
        fs::create_dir(path).unwrap();
    }
    fs::create_dir(cwd.join(".archon")).unwrap();
    fs::write(cwd.join(".archon/config.toml"), "invalid = [").unwrap();
    fs::write(root.join("out.json"), "{}").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_archon"));
    for key in [
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "TMPDIR",
        "TMP",
        "TEMP",
    ] {
        let path = temporary.path().join(key);
        fs::create_dir(&path).unwrap();
        command.env(key, path);
    }
    let before = snapshot(temporary.path());
    let home_permissions = fs::metadata(&home).unwrap().permissions();
    let root_permissions = fs::metadata(&root).unwrap().permissions();
    let evidence_file = root.join("out.json");
    let evidence_permissions = fs::metadata(&evidence_file).unwrap().permissions();
    let restore = RestorePermissions(vec![
        (home.clone(), home_permissions),
        (root.clone(), root_permissions),
        (evidence_file.clone(), evidence_permissions.clone()),
    ]);
    let mut permissions = evidence_permissions.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&evidence_file, permissions).unwrap();
    let mut permissions = fs::metadata(&home).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&home, permissions).unwrap();
    let mut permissions = fs::metadata(&root).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&root, permissions).unwrap();
    let output = command
        .current_dir(&cwd)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .args(["workflow", "verify-deliverable", "--root"])
        .arg(&root)
        .args([
            "--contract-json",
            r#"{"kind":"evidence","artifact_path":"out.json"}"#,
        ])
        .output()
        .unwrap();
    let after = snapshot(temporary.path());
    drop(restore);
    verdict(&output, 0);
    assert_eq!(after, before);
}

#[test]
fn stamped_command_keeps_contract_json_valid() {
    initialize();
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("root with space and 'quote");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join(".archon")).unwrap();
    fs::write(root.join(".archon/out.json"), "{}").unwrap();
    let root_text = root.to_string_lossy().into_owned();
    let command = deliverable_contract::verification_command(
        &ContractRoots::project_only(&root_text),
        &json!({"kind":"evidence","artifact_path":".archon/out.json"}),
    );
    let mut item =
        json!({"artifact_requirements":[".archon/out.json"],"focused_verification":command});
    archon_workflow::v2::project_artifact_stamping::stamp_project_artifact_paths(
        item.as_object_mut().unwrap(),
        &root_text,
    );
    verdict(&shell(item["focused_verification"].as_str().unwrap()), 0);
}

#[test]
fn backslash_roots_render_as_runnable_forward_slashes() {
    initialize();
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("out.json"), "{}").unwrap();
    let backslashes = root.path().to_string_lossy().replace('/', "\\");
    let command = deliverable_contract::verification_command(
        &ContractRoots::project_only(backslashes),
        &json!({"kind":"evidence","artifact_path":"out.json"}),
    );
    verdict(&shell(&command), 0);
}

#[test]
fn unset_and_error_program_paths_fail_closed() {
    if let Ok(mode) = std::env::var("FLOOR_PROGRAM_FAILURE_CASE") {
        if mode == "error" {
            deliverable_contract::set_verifier_program(Err("exact startup path failure".into()));
        }
        let command = deliverable_contract::verification_command(
            &ContractRoots::project_only("."),
            &json!({"kind":"evidence","artifact_path":"out.json"}),
        );
        assert!(command.starts_with("printf"));
        assert!(command.contains(if mode == "error" {
            "exact startup path failure"
        } else {
            "verifier program has not been recorded at startup"
        }));
        let output = shell(&command);
        assert_eq!(output.status.code(), Some(1));
        return;
    }
    for mode in ["unset", "error"] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "unset_and_error_program_paths_fail_closed",
                "--nocapture",
            ])
            .env("FLOOR_PROGRAM_FAILURE_CASE", mode)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn malformed_nonfinite_tokens_are_not_rewritten_as_numbers() {
    let root = tempfile::tempdir().unwrap();
    for content in [
        "1NaN",
        "NaN1",
        "-NaN",
        "NaN",
        "Infinity",
        "-Infinity",
        "{\"value\":NaN}",
    ] {
        fs::write(root.path().join("out.json"), content).unwrap();
        let output = cli(
            root.path(),
            r#"{"kind":"evidence","artifact_path":"out.json"}"#,
        );
        let value = verdict(&output, 1);
        assert!(
            value["failures"][0]
                .as_str()
                .unwrap()
                .contains("not valid JSON"),
            "{content}: {value}"
        );
    }
}

#[test]
fn native_predicates_fail_before_agent_typed_command_runs() {
    deliverable_contract::set_verifier_program(Ok(env!("CARGO_BIN_EXE_archon").into()));
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("marker");
    let command = format!(
        "printf ran > '{}'",
        marker
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "'\"'\"'")
    );
    let contract =
        json!({"kind":"evidence","artifact_path":"missing.json","typed_verifier_command":command});
    let roots = ContractRoots::project_only(directory.path().to_string_lossy());
    let output = shell(&deliverable_contract::verification_command(
        &roots, &contract,
    ));
    verdict(&output, 1);
    assert!(!marker.exists());
    fs::write(directory.path().join("missing.json"), "{}").unwrap();
    let output = shell(&deliverable_contract::verification_command(
        &roots, &contract,
    ));
    verdict(&output, 0);
    assert_eq!(fs::read_to_string(marker).unwrap(), "ran");
}
