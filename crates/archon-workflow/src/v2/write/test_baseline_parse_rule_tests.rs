use super::*;
#[test]
fn quiet_summaries_are_verdicts_only_when_complete() {
    for output in [
        "running 0 tests\ntest result: ok. 0 passed; 0 failed",
        "running 1 test\n.\ntest result: ok. 1 passed; 0 failed",
        "running 3 tests\nF..\ntest result: FAILED. 2 passed; 1 failed",
    ] {
        assert!(harness_reported(output), "{output}");
    }
    for output in [
        "running 1 tests",
        "test result: ok. 1 passed; 0 failed",
        "running 1 tests\ntest result: ok. 1 passed; 0 failed\nrunning 2 tests",
    ] {
        assert!(!harness_reported(output), "{output}");
    }
}
#[test]
fn format_outputs_attribute_existing_files() {
    let temp = tempfile::tempdir().unwrap();
    for path in ["src/a.go", "src/b.ts", "src/c.py"] {
        std::fs::create_dir_all(temp.path().join(path).parent().unwrap()).unwrap();
        std::fs::write(temp.path().join(path), "content").unwrap();
    }
    for (output, expected) in [
        ("src/a.go\n", "src/a.go"),
        ("Diff in src/a.go:2:\n", "src/a.go"),
    ] {
        assert_eq!(
            diagnostic_files_for(output, temp.path(), &format!("check {expected}")),
            [expected]
        );
    }
    assert!(diagnostic_files("missing.go:2:3", temp.path()).is_empty());
}
#[test]
fn real_result_ids_are_preserved() {
    assert_eq!(
        failing_tests("test store::tests::stale ... FAILED"),
        ["store::tests::stale"]
    );
}

#[test]
fn real_harness_output_keeps_verdicts_at_each_verbosity() {
    let module = module_path!().split_once("::").unwrap().1;
    let child = format!("{module}::quiet_summaries_are_verdicts_only_when_complete");
    for verbosity in [Some("--quiet"), Some("-q"), None] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", &child]);
        command.args(verbosity);
        let output = command.output().unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(output.status.success(), "{text}");
        assert!(text.contains("running 1 test"), "{text}");
        assert!(harness_reported(&text), "{text}");
        assert_eq!(failed_count(&text), Some(0), "{text}");
    }
}

#[test]
fn recorded_formatter_outputs_keep_file_attribution() {
    let metadata: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/diagnostic_outputs/provenance.json"
    ))
    .unwrap();
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("src")).unwrap();
    for (output, file) in [
        (
            include_str!("../../../../../tests/fixtures/diagnostic_outputs/gofmt.stdout.txt"),
            "src/a.go",
        ),
        (
            include_str!("../../../../../tests/fixtures/diagnostic_outputs/rustfmt.stdout.txt"),
            "src/d.rs",
        ),
    ] {
        std::fs::write(temp.path().join(file), "content").unwrap();
        let output = output.replace(
            metadata["cwd"].as_str().unwrap(),
            &temp.path().to_string_lossy(),
        );
        assert_eq!(
            diagnostic_files_for(&output, temp.path(), &format!("check {file}")),
            [file]
        );
    }
}

#[test]
fn baseline_failure_identity_preserves_recorded_parameters_subcases_and_paths() {
    let fixture = |name: &str| {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/diagnostic_outputs")
                .join(name),
        )
        .unwrap()
    };
    let root = std::path::Path::new("/private/tmp/s8b-r6-recorded/base");
    for (name, first, second) in [
        (
            "pytest-parameters.txt",
            "test_add[1-2-3]",
            "test_add[2-3-5]",
        ),
        ("go-subcases.txt", "TestParse/case_1", "TestParse/case_2"),
        (
            "pytest-hex-parameters.txt",
            "test_hex[0x1]",
            "test_hex[0x2]",
        ),
    ] {
        let output = fixture(name);
        let marker = |id: &str| {
            output
                .lines()
                .find(|line| line.contains(id) && crate::failure_evidence::is_failure_line(line))
                .unwrap()
        };
        assert_ne!(
            failure_identity(marker(first), root),
            failure_identity(marker(second), root)
        );
    }
    assert_ne!(
        failure_identity(&fixture("rustc-hex-one.txt"), root),
        failure_identity(&fixture("rustc-hex-two.txt"), root)
    );
    assert_ne!(
        failure_identity(&fixture("rustc-long-a.txt"), root),
        failure_identity(&fixture("rustc-long-b.txt"), root)
    );
    let a = fixture("rustc-a.txt");
    assert_ne!(
        failure_identity(&a, root),
        failure_identity(&fixture("rustc-b.txt"), root)
    );
    assert_eq!(
        failure_identity(&a, root),
        failure_identity(
            &fixture("rustc-tip-a.txt"),
            std::path::Path::new("/private/tmp/s8b-r6-recorded/tip")
        )
    );
    let go = fixture("go-subcases.txt");
    assert_eq!(
        failure_identity(&go, root),
        failure_identity(&fixture("go-subcases-slower.txt"), root)
    );
}
#[test]
fn baseline_incidental_file_tokens_stay_unresolved() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("Makefile"), "").unwrap();
    assert!(
        diagnostic_files_for("[Makefile:3: test] Error 1", root.path(), "make test").is_empty()
    );
    assert!(diagnostic_files_for("Makefile", root.path(), "make test").is_empty());
    assert_eq!(
        diagnostic_files_for("Makefile", root.path(), "check Makefile"),
        ["Makefile"]
    );
}

#[test]
#[ignore = "child harness error-return fixture"]
fn unlocated_error_child() -> Result<(), String> {
    Err("recorded error without a panic location".into())
}

#[test]
fn real_error_return_keeps_the_red_id_without_inventing_a_location() {
    let child = format!(
        "{}::unlocated_error_child",
        module_path!().split_once("::").unwrap().1
    );
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", &child])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let summary = text
        .lines()
        .find(|line| failed_count(line).is_some())
        .unwrap();
    assert_eq!(failure_identity(summary, std::path::Path::new("")), None);
    assert_eq!(failing_tests(&text), [child.clone()]);
    assert!(
        super::super::test_baseline_run_base::panic_files(&text, &child, std::path::Path::new(""))
            .is_empty()
    );
}

#[test]
fn baseline_aggregate_counts_are_not_an_unnamed_failure_identity() {
    let output =
        include_str!("../../../../../tests/fixtures/diagnostic_outputs/pytest-parameters.txt");
    let summary = output.lines().last().unwrap();
    assert!(crate::failure_evidence::is_failure_line(summary));
    assert_eq!(failure_identity(summary, std::path::Path::new("")), None);
    assert!(failure_identity(output, std::path::Path::new("")).is_some());
    let output = include_str!("../../../../../tests/fixtures/diagnostic_outputs/rustc-a.txt");
    assert_eq!(
        failure_identity(output.lines().last().unwrap(), std::path::Path::new("")),
        None
    );
    assert!(failure_identity(output, std::path::Path::new("")).is_some());
}

#[test]
fn repository_prefix_spellings_match_on_every_host() {
    let root = std::path::Path::new(r"\\?\C:\Work\Base");
    let pattern = identity_root_pattern(root).unwrap();
    for path in [
        r"C:\Work\Base\src/case_12.rs",
        "c:/work/base/src/case_12.rs",
        r"\\?\c:\work\base\src/case_12.rs",
        r"C:\Work/Base\src/case_12.rs",
    ] {
        assert_eq!(
            pattern.replace_all(&format!("error: {path}:3:5"), "${before}<root>/"),
            "error: <root>/src/case_12.rs:3:5",
            "{path}"
        );
    }
    for other in [
        r"D:\Work\Base\src/a.rs",
        "C:/Work/Basement/src/a.rs",
        "src/case_12.rs",
    ] {
        assert!(!pattern.is_match(other), "{other}");
    }
    let network =
        identity_root_pattern(std::path::Path::new(r"\\?\UNC\server\share\base")).unwrap();
    assert!(network.is_match(r"\\SERVER\share\base\src/a.rs"));
    assert!(network.is_match(r"\\?\UNC\server\share\base\src/a.rs"));
    let sensitive = identity_root_pattern(std::path::Path::new("/work/base")).unwrap();
    assert!(!sensitive.is_match("/WORK/base/src/a.rs"));
}

#[test]
fn recorded_failure_identity_masks_native_root_spellings() {
    let recorded = include_str!("../../../../../tests/fixtures/diagnostic_outputs/rustc-a.txt");
    let recorded_root = "/private/tmp/s8b-r6-recorded/base";
    let before = recorded.replace(recorded_root, r"C:\Work\Base");
    let after = recorded.replace(recorded_root, r"\\?\c:\work\tip");
    let expected = failure_identity(recorded, std::path::Path::new(recorded_root))
        .expect("the recorded diagnostic identifies a failure");
    for (output, root) in [(&before, r"C:\Work\Base"), (&after, r"C:\Work\Tip")] {
        assert_eq!(
            failure_identity(output, std::path::Path::new(root)).as_deref(),
            Some(expected.as_str())
        );
    }
}
