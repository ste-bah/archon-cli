//! Tests for Compilation + Orphan Detection Gates (TASK-PIPE-E06).
//!
//! Validates: CompilationGate success/failure, OrphanDetectionGate
//! reference detection, GateResult/GateFailure types, gate persistence.

use archon_pipeline::coding::gates::{
    CompilationGate, GateFailure, GateResultRecord, Language, OrphanDetectionGate,
    load_gate_result, save_gate_result,
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

mod compilation_tests {
    use super::*;

    #[tokio::test]
    async fn compilation_success_on_valid_project() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        // Create a valid Rust project
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"test-proj\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "pub fn hello() -> &'static str { \"hello\" }\n",
        )
        .unwrap();

        let gate = CompilationGate;
        let result = gate.run(tmp.path(), Language::Rust).await;
        assert!(
            result.gate_passed,
            "valid project should compile: {:?}",
            result.failures
        );
        assert_eq!(result.gate_name, "compilation");
    }

    #[tokio::test]
    async fn compilation_failure_on_syntax_error() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"bad-proj\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        // Intentional syntax error
        std::fs::write(src.join("lib.rs"), "pub fn broken( { }\n").unwrap();

        let gate = CompilationGate;
        let result = gate.run(tmp.path(), Language::Rust).await;
        assert!(!result.gate_passed, "syntax error should fail compilation");
        assert!(!result.failures.is_empty());
        assert!(
            !result.evidence.is_empty(),
            "should capture compiler output"
        );
    }

    #[tokio::test]
    async fn compilation_captures_error_output() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"err-proj\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "fn main() { let x: i32 = \"not_int\"; }\n",
        )
        .unwrap();

        let gate = CompilationGate;
        let result = gate.run(tmp.path(), Language::Rust).await;
        assert!(!result.gate_passed);
        // Evidence should contain compiler error text
        assert!(
            result.evidence.contains("error") || result.evidence.contains("mismatched"),
            "evidence should contain compiler error info, got: {}",
            &result.evidence[..result.evidence.len().min(500)]
        );
    }
}

mod orphan_tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn orphan_detected_for_unreferenced_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        // Main file does NOT reference orphan
        std::fs::write(src.join("main.rs"), "fn main() {}\n").unwrap();
        // Orphan file — not referenced anywhere
        std::fs::write(src.join("orphan.rs"), "pub fn orphan_fn() {}\n").unwrap();

        let gate = OrphanDetectionGate;
        let new_files = vec![src.join("orphan.rs")];
        let result = gate.run(&new_files, tmp.path()).await;

        assert!(!result.gate_passed, "unreferenced file should be orphan");
        assert!(
            result
                .failures
                .iter()
                .any(|f| f.file.as_deref().unwrap_or("").contains("orphan")),
            "failure should name orphan file"
        );
    }

    #[tokio::test]
    async fn no_orphan_when_file_is_referenced() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        // Main file references utils via mod
        std::fs::write(
            src.join("main.rs"),
            "mod utils;\nfn main() { utils::helper(); }\n",
        )
        .unwrap();
        std::fs::write(src.join("utils.rs"), "pub fn helper() {}\n").unwrap();

        let gate = OrphanDetectionGate;
        let new_files = vec![src.join("utils.rs")];
        let result = gate.run(&new_files, tmp.path()).await;

        assert!(
            result.gate_passed,
            "referenced file should not be orphan: {:?}",
            result.failures
        );
    }

    #[tokio::test]
    async fn orphan_detects_use_import_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(src.join("lib.rs"), "pub mod api;\nuse api::Router;\n").unwrap();
        std::fs::write(src.join("api.rs"), "pub struct Router;\n").unwrap();

        let gate = OrphanDetectionGate;
        let new_files = vec![src.join("api.rs")];
        let result = gate.run(&new_files, tmp.path()).await;

        assert!(result.gate_passed, "file referenced via use should pass");
    }

    #[tokio::test]
    async fn orphan_empty_new_files_passes() {
        let tmp = tempfile::tempdir().unwrap();

        let gate = OrphanDetectionGate;
        let result = gate.run(&[], tmp.path()).await;
        assert!(result.gate_passed, "no new files means no orphans");
    }

    #[tokio::test]
    async fn orphan_reports_evidence_for_referenced_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        std::fs::write(src.join("main.rs"), "mod handler;\nfn main() {}\n").unwrap();
        std::fs::write(src.join("handler.rs"), "pub fn handle() {}\n").unwrap();

        let gate = OrphanDetectionGate;
        let new_files = vec![src.join("handler.rs")];
        let result = gate.run(&new_files, tmp.path()).await;

        assert!(result.gate_passed);
        assert!(
            result.evidence.contains("handler") || result.evidence.contains("main"),
            "evidence should mention referencing file(s)"
        );
    }
}

mod gate_result_tests {
    use super::*;

    #[test]
    fn gate_result_serialization_roundtrip() {
        let result = GateResultRecord {
            gate_name: "compilation".into(),
            gate_passed: true,
            evidence: "cargo build exit 0".into(),
            failures: vec![],
            timestamp: "2026-04-07T12:00:00Z".into(),
        };

        let json = serde_json::to_string_pretty(&result).unwrap();
        let deserialized: GateResultRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.gate_name, "compilation");
        assert!(deserialized.gate_passed);
    }

    #[test]
    fn gate_failure_captures_file_and_details() {
        let failure = GateFailure {
            description: "Orphaned file".into(),
            file: Some("src/orphan.rs".into()),
            details: "No references found in project".into(),
        };

        let json = serde_json::to_string(&failure).unwrap();
        let deserialized: GateFailure = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.file, Some("src/orphan.rs".into()));
    }

    #[test]
    fn gate_result_persistence_roundtrip() {
        let result = GateResultRecord {
            gate_name: "orphan-detection".into(),
            gate_passed: false,
            evidence: "src/orphan.rs has no references".into(),
            failures: vec![GateFailure {
                description: "Orphaned file".into(),
                file: Some("src/orphan.rs".into()),
                details: "zero references found".into(),
            }],
            timestamp: "2026-04-07T12:00:00Z".into(),
        };

        let tmp = tempfile::tempdir().unwrap();
        save_gate_result(&result, tmp.path()).unwrap();
        let loaded = load_gate_result(&result.gate_name, tmp.path()).unwrap();

        assert_eq!(loaded.gate_name, "orphan-detection");
        assert!(!loaded.gate_passed);
        assert_eq!(loaded.failures.len(), 1);
    }
}
