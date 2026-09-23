use super::*;

fn commands(body: &str) -> Vec<(String, bool)> {
    zero_match_commands(body)
        .into_iter()
        .map(|entry| (entry.command, entry.presented_as_passing))
        .collect()
}

#[test]
fn ordinary_filtered_rust_test_summary_is_not_zero_matched() {
    assert!(!output_reports_zero_matched(
        "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1007 filtered out."
    ));
}

#[test]
fn all_filtered_rust_test_summary_is_zero_matched() {
    assert!(output_reports_zero_matched(
        "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1007 filtered out."
    ));
    assert!(output_reports_zero_matched("running 0 tests"));
}

#[test]
fn mixed_harness_output_with_real_match_is_zero_matched_per_line() {
    // A multi-harness run where one harness matched nothing: the line says so.
    let body = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 42 filtered out.\n\
                test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out.";
    assert!(output_reports_zero_matched(body));
}

#[test]
fn prose_about_zero_tests_is_not_runner_output() {
    for prose in [
        "data_store ingest families and coverage_tests compile zero tests; reached via #[path]",
        "250 tests listed; used to verify module reachability (0 ingest/coverage_tests entries)",
        "the filter matched zero tests last week, fixed now: 3 passed; 0 failed",
        "0-test modules are documented in the audit",
        "matches zero tests is the phrase the old detector keyed on",
        "Summary: 10 tests run: 10 passed",
    ] {
        assert!(!output_reports_zero_matched(prose), "{prose}");
    }
}

#[test]
fn other_runner_zero_forms_are_detected() {
    for output in [
        "Starting 0 tests across 3 binaries (250 skipped)",
        "     Summary [   0.005s] 0 tests run: 0 passed, 0 skipped",
        "============ no tests ran in 0.01s ============",
        "collected 0 items",
        "No tests found, exiting with code 1",
        "testing: warning: no tests to run",
        "ok  \tpkg/foo\t0.001s [no tests to run]",
        "  0 passing (2ms)",
    ] {
        assert!(output_reports_zero_matched(output), "{output}");
    }
}

/// Issue-82, verbatim from a verification branch whose two declared commands
/// carried these as their whole `output_summary`. Neither the go-specific
/// spellings nor the counted `running 0 tests` form could match them, so both
/// excusal rules downstream stayed shut and a correct branch was demoted.
#[test]
fn a_verifiers_own_zero_match_summary_is_recognised() {
    for output in [
        "0 tests run: 356 skipped, 'error: no tests to run' (exit 1). NO test FAILED and no test \
         name appears in the output — the verbatim shallow filter strin",
        "0 tests run: 356 skipped, 'error: no tests to run' (exit 1). NO test FAILED — same \
         pre-existing filter-resolution mismatch: ",
        // Each half on its own, so neither is carried by the other.
        "exit 1: 'error: no tests to run' for the declared filter",
        "0 tests run",
        "0 tests matched the declared filter",
    ] {
        assert!(output_reports_zero_matched(output), "{output}");
    }
}

/// The digit-boundary trap: every one of these CONTAINS `0 tests run` or
/// `0 tests matched` as a substring while reporting real work.
#[test]
fn a_count_that_merely_ends_in_zero_is_not_zero_matched() {
    for output in [
        "10 tests run: 3 skipped",
        "100 tests matched",
        "Summary [ 0.005s] 20 tests run: 20 passed, 0 skipped",
        "1230 tests matched the filter",
        "test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out.",
    ] {
        assert!(!output_reports_zero_matched(output), "{output}");
    }
}

#[test]
fn listing_and_build_invocations_are_never_candidates() {
    for command in [
        "cargo test -p archon-trading --lib -- --list",
        "cargo test -p demo --no-run",
        "cargo test --help",
        "cargo --version",
        "pytest --dry-run tests/",
        "cargo check -p demo && cargo clippy -p demo",
        "cargo build --release",
        "cargo fmt --all -- --check",
        "go vet ./...",
    ] {
        assert!(command_is_non_run_invocation(command), "{command}");
    }
    for command in [
        "cargo test -p demo some_filter",
        "cargo build -p demo && cargo test -p demo some_filter",
        "cargo nextest run -p demo",
        "pytest tests/test_demo.py -k missing",
        "go test ./... -run Missing",
    ] {
        assert!(!command_is_non_run_invocation(command), "{command}");
    }
}

#[test]
fn json_walk_names_zero_match_commands_and_how_they_are_presented() {
    let body = r#"{
        "commands_run": [
            {"command": "cargo fmt --all -- --check", "status": "succeeded", "output_summary": "running 0 tests"},
            {"command": "cargo test -p demo --lib -- --list", "status": "succeeded",
             "output_summary": "250 tests listed (0 ingest entries; running 0 tests is fine here)"},
            {"command": "cargo test -p wrong-crate some_test -- --nocapture", "status": "succeeded",
             "output_summary": "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 42 filtered out."},
            {"command": "cargo test -p demo stale_name", "status": "failed", "exit_code": 101,
             "output_summary": "running 0 tests"},
            {"command": "cargo test -p demo unmarked", "output_summary": "running 0 tests"},
            {"command": "cargo test -p demo by_exit", "exit_code": 1, "output_summary": "running 0 tests"}
        ]
    }"#;
    assert_eq!(
        commands(body),
        vec![
            (
                "cargo test -p wrong-crate some_test -- --nocapture".to_string(),
                true
            ),
            ("cargo test -p demo stale_name".to_string(), false),
            ("cargo test -p demo unmarked".to_string(), true),
            ("cargo test -p demo by_exit".to_string(), false),
        ]
    );
}

#[test]
fn text_body_pairs_runner_summary_with_preceding_command_line() {
    let body = "ran: cargo test some_filter\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out.";
    let found = commands(body);
    assert_eq!(found.len(), 1);
    assert!(found[0].0.contains("cargo test some_filter"), "{found:?}");
    assert!(found[0].0.contains("0 passed"), "{found:?}");
    assert!(found[0].1);
}

#[test]
fn text_body_without_zero_match_names_nothing() {
    assert!(
        commands("test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out.")
            .is_empty()
    );
    assert!(commands("ran: cargo test -- --list\nrunning 0 tests").is_empty());
}

#[test]
fn declared_match_follows_the_read_guard_containment_rule() {
    let declared = vec![
        "cargo test -p demo  focused_case".to_string(),
        "".to_string(),
        "pytest tests/test_demo.py -k focused".to_string(),
    ];
    assert!(command_matches_declared(
        "cargo test -p demo focused_case -- --nocapture",
        &declared
    ));
    assert!(command_matches_declared(
        "cd crates && pytest tests/test_demo.py -k focused",
        &declared
    ));
    // An agent's abbreviation of a declared command still counts.
    assert!(command_matches_declared("cargo test -p demo", &declared));
    assert!(!command_matches_declared(
        "cargo test -p other focused_case",
        &declared
    ));
    assert!(!command_matches_declared("cargo", &declared));
    assert!(!command_matches_declared("", &declared));
}

#[test]
fn evidence_clause_is_bounded() {
    let found: Vec<ZeroMatchCommand> = (0..5)
        .map(|index| ZeroMatchCommand {
            command: format!("cargo test -p demo case_{index}"),
            presented_as_passing: true,
        })
        .collect();
    let evidence = zero_match_evidence(&found);
    assert!(evidence.starts_with("offending test command(s): `cargo test -p demo case_0`"));
    assert!(
        evidence.contains("case_2") && !evidence.contains("case_3"),
        "{evidence}"
    );
}
