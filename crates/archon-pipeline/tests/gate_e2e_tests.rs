//! Integration tests for TestsRunGate, E2ESmokeTestGate, ManualOverride, and fraud detection.
//! REQ-IMPROVE-010, REQ-IMPROVE-019, EC-PIPE-010

use archon_pipeline::coding::gates::{
    save_gate_result, load_gate_result, E2ESmokeTestGate, Language, ManualOverride, TestsRunGate,
};
use std::path::PathBuf;
use tempfile::TempDir;

// ===========================================================================
// is_test_only_evidence — fraud detection unit tests
// ===========================================================================

#[test]
fn test_only_evidence_rust_result_ok() {
    let output = "test result: ok. 42 passed, 0 failed";
    assert!(
        E2ESmokeTestGate::is_test_only_evidence(output),
        "Should detect 'test result: ok. 42 passed, 0 failed' as test-only"
    );
}

#[test]
fn test_only_evidence_tests_n_passed() {
    let output = "Tests: 10 passed";
    assert!(
        E2ESmokeTestGate::is_test_only_evidence(output),
        "Should detect 'Tests: 10 passed' as test-only"
    );
}

#[test]
fn test_only_evidence_passed_failed_counts() {
    let output = "42 passed, 0 failed";
    assert!(
        E2ESmokeTestGate::is_test_only_evidence(output),
        "Should detect '42 passed, 0 failed' as test-only"
    );
}

#[test]
fn test_only_evidence_http_response_not_test() {
    let output = "HTTP/1.1 200 OK\n{\"status\": \"healthy\"}";
    assert!(
        !E2ESmokeTestGate::is_test_only_evidence(output),
        "HTTP 200 response should NOT be flagged as test-only"
    );
}

#[test]
fn test_only_evidence_compile_output_not_test() {
    let output = "archon kb compile --all\nCompiled 20 documents";
    assert!(
        !E2ESmokeTestGate::is_test_only_evidence(output),
        "Compile output should NOT be flagged as test-only"
    );
}

// ===========================================================================
// ManualOverride — gate_passed=true with justification in evidence
// ===========================================================================

#[tokio::test]
async fn manual_override_produces_gate_passed() {
    let gate = E2ESmokeTestGate;
    let project_root = PathBuf::from("/tmp");
    let override_info = ManualOverride {
        justification: "Feature requires hardware not available in CI".into(),
        overridden_by: "test-engineer".into(),
        timestamp: "2026-04-07T00:00:00Z".into(),
    };

    let result = gate
        .run(&project_root, "echo smoke", Some(override_info))
        .await;

    assert!(result.gate_passed, "Manual override should produce gate_passed=true");
    assert!(
        result.evidence.contains("MANUAL OVERRIDE"),
        "Evidence should contain MANUAL OVERRIDE marker"
    );
    assert!(
        result.evidence.contains("Feature requires hardware not available in CI"),
        "Evidence should contain the justification"
    );
}

// ===========================================================================
// TestsRunGate — exit 0 passes, non-zero fails
// ===========================================================================

#[tokio::test]
async fn tests_run_gate_exit_zero_passes() {
    // Create a minimal Rust project with a trivial passing test so cargo test exits 0.
    let tmp = TempDir::new().expect("tempdir");
    let project_root = tmp.path();

    // Write minimal Cargo.toml
    std::fs::write(
        project_root.join("Cargo.toml"),
        "[package]\nname = \"gate_test_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");

    let src_dir = project_root.join("src");
    std::fs::create_dir_all(&src_dir).expect("create src");
    // Write lib.rs with a trivially passing test
    std::fs::write(
        src_dir.join("lib.rs"),
        "#[test]\nfn it_works() { assert_eq!(2 + 2, 4); }\n",
    )
    .expect("write lib.rs");

    let gate = TestsRunGate;
    let result = gate.run(project_root, Language::Rust).await;

    assert!(
        result.gate_passed,
        "Trivial passing test should yield gate_passed=true; evidence: {}",
        result.evidence
    );
    assert_eq!(result.gate_name, "tests-run");
}

#[tokio::test]
async fn tests_run_gate_nonexistent_project_fails() {
    let gate = TestsRunGate;
    // Run in a directory that has no Cargo.toml — cargo test should fail
    let bogus_root = PathBuf::from("/tmp/no_such_project_xyz_archon_test");
    let result = gate.run(&bogus_root, Language::Rust).await;
    // Either failed to execute (no such dir) or cargo returned non-zero
    assert!(
        !result.gate_passed,
        "Tests in nonexistent/empty directory should fail"
    );
    assert_eq!(result.gate_name, "tests-run");
}

// ===========================================================================
// E2ESmokeTestGate — rejects test-only evidence even when command exits 0
// ===========================================================================

#[tokio::test]
async fn e2e_gate_rejects_test_only_evidence_even_on_exit_zero() {
    // Create a temp script that outputs something that looks like test output
    let tmp = TempDir::new().expect("tempdir");
    let script_path = tmp.path().join("fake_smoke.sh");
    std::fs::write(
        &script_path,
        "#!/bin/sh\nprintf 'test result: ok. 5 passed, 0 failed\\n'\n",
    )
    .expect("write script");

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();
    }

    let gate = E2ESmokeTestGate;
    let cmd = script_path.to_string_lossy().to_string();
    let result = gate.run(tmp.path(), &cmd, None).await;

    assert!(
        !result.gate_passed,
        "E2E gate should reject test-only evidence: {}",
        result.evidence
    );
    assert!(
        result.failures.iter().any(|f| f.description.contains("FRAUD DETECTED")),
        "Failure description should mention FRAUD DETECTED"
    );
}

// ===========================================================================
// Persistence — gate results save and load from .pipeline-state/<session>/gate-results/
// ===========================================================================

#[test]
fn gate_results_persist_and_load() {
    use archon_pipeline::coding::gates::GateResultRecord;

    let tmp = TempDir::new().expect("tempdir");
    let session_dir = tmp.path().join(".pipeline-state").join("test-session-001");

    let record = GateResultRecord {
        gate_name: "e2e-smoke".into(),
        gate_passed: true,
        evidence: "STDOUT:\nService started on :8080\nSTDERR:\n".into(),
        failures: vec![],
        timestamp: "1712450000Z".into(),
    };

    save_gate_result(&record, &session_dir).expect("save should succeed");

    let loaded = load_gate_result("e2e-smoke", &session_dir).expect("load should succeed");

    assert_eq!(loaded.gate_name, "e2e-smoke");
    assert!(loaded.gate_passed);
    assert_eq!(loaded.evidence, record.evidence);
    assert_eq!(loaded.timestamp, record.timestamp);
}

#[test]
fn tests_run_gate_result_persists_and_loads() {
    use archon_pipeline::coding::gates::{GateFailure, GateResultRecord};

    let tmp = TempDir::new().expect("tempdir");
    let session_dir = tmp.path().join(".pipeline-state").join("test-session-002");

    let record = GateResultRecord {
        gate_name: "tests-run".into(),
        gate_passed: false,
        evidence: "STDOUT:\n\nSTDERR:\nerror[E0308]: mismatched types".into(),
        failures: vec![GateFailure {
            description: "Test suite failed".into(),
            file: None,
            details: "error[E0308]: mismatched types".into(),
        }],
        timestamp: "1712450001Z".into(),
    };

    save_gate_result(&record, &session_dir).expect("save should succeed");

    let loaded = load_gate_result("tests-run", &session_dir).expect("load should succeed");

    assert_eq!(loaded.gate_name, "tests-run");
    assert!(!loaded.gate_passed);
    assert_eq!(loaded.failures.len(), 1);
    assert_eq!(loaded.failures[0].description, "Test suite failed");
}
