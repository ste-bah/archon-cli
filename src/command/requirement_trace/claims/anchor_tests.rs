//! #442: a path span that names something inside its file (`path::symbol`,
//! `path:12-30`, `path:291/319`) is a path plus an anchor, and a path written
//! relative to a package is read against the packages it can name.

use super::observe::{self, Anchor, Span};
use super::tests::{commit, task, task_set};
use super::{ClaimTrace, falsify_claims};

/// A workspace of two packages that both have `tests/shared.rs` (each with
/// its own symbol) and one file only the first package has. The root has its
/// own `src/` and `tests/`, so a package-relative path starts like a root one.
const WORKSPACE: &[(&str, &str)] = &[
    ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
    ("src/main.rs", "fn main() {}\n"),
    ("tests/root.rs", "#[test]\nfn root_case() {}\n"),
    (
        "crates/one/Cargo.toml",
        "[package]\nname = \"one\"\nversion = \"0.1.0\"\n",
    ),
    ("crates/one/src/lib.rs", "pub fn build_record() {}\n"),
    (
        "crates/one/src/checks/tests.rs",
        "#[test]\nfn stale_claim_is_an_error() {}\n",
    ),
    (
        "crates/one/tests/shared.rs",
        "#[test]\nfn shared_one() {}\n\n\n\n",
    ),
    (
        "crates/two/Cargo.toml",
        "[package]\nname = \"two\"\nversion = \"0.1.0\"\n",
    ),
    ("crates/two/src/lib.rs", "pub fn other() {}\n"),
    (
        "crates/two/tests/shared.rs",
        "#[test]\nfn shared_two() {}\n",
    ),
    (
        "crates/two/tests/gate_refusals.rs",
        "#[test]\nfn refuses() {}\n",
    ),
    // A directory whose name is a glob that also matches its sibling.
    ("data/lit[1]/a.rs", "pub fn plain_item() {}\n"),
    ("data/lit1/b.rs", "pub fn only_in_sibling() {}\n"),
];

/// Declares a file of package `one`, so `one` is the task's area.
const IN_ONE: &str =
    "## Files Expected to Change\n\n- `crates/one/src/lib.rs` — exists (1 line)\n\n";
/// Declares only a new top-level file, so the task has no package area.
const NO_AREA: &str = "## Files Expected to Change\n\n- `notes/plan.txt` — new\n\n";

fn run_line(declares: &str, line: &str) -> ClaimTrace {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let base = commit(&root, WORKSPACE);
    let body = task(
        "TASK-X-010",
        "REQ-X-001",
        &format!("{declares}## Acceptance Criteria\n\n- REQ-X-001 {line}\n"),
    );
    let task_dir = task_set(dir.path(), &root, &base, &[("TASK-X-010", &body)]);
    let (bindings, findings) =
        crate::command::requirement_trace::load_bindings_with_findings(&task_dir).unwrap();
    assert!(findings.is_empty(), "{findings:?}");
    falsify_claims(&task_dir, bindings).expect("falsify")
}

fn finding_texts(trace: &ClaimTrace) -> Vec<&str> {
    trace.findings.iter().map(|f| f.text.as_str()).collect()
}

fn assert_supported(trace: &ClaimTrace) {
    assert!(trace.findings.is_empty(), "{}", trace.report);
    assert!(
        trace
            .report
            .contains("1 claim(s): 1 tested, 0 refuted, 0 untestable"),
        "{}",
        trace.report
    );
}

#[test]
fn issue_442_path_symbol_whose_file_has_the_symbol_is_supported() {
    let trace = run_line(IN_ONE, "is built by `crates/one/src/lib.rs::build_record`.");
    assert_supported(&trace);
    assert!(
        trace.report.contains("observables: 3 exist"),
        "path, anchor and declared file all exist: {}",
        trace.report
    );
}

#[test]
fn issue_442_path_symbol_whose_file_lacks_the_symbol_is_refuted() {
    // `other` is a word of the repository, in another file: a span that
    // binds the symbol to this file is not satisfied by that.
    let trace = run_line(IN_ONE, "is built by `crates/two/tests/shared.rs::other`.");
    let texts = finding_texts(&trace);
    assert_eq!(texts.len(), 1, "{}", trace.report);
    assert!(
        texts[0].contains("`crates/two/tests/shared.rs::other`: `other` is not in `crates/two/tests/shared.rs` at base commit"),
        "{}",
        texts[0]
    );
}

#[test]
fn issue_442_crate_relative_path_with_a_unique_suffix_is_read_in_that_crate() {
    let trace = run_line(
        IN_ONE,
        "is proven by `src/checks/tests.rs::stale_claim_is_an_error`.",
    );
    assert_supported(&trace);
    let missing = run_line(IN_ONE, "is proven by `src/checks/tests.rs::ghost_case`.");
    let texts = finding_texts(&missing);
    assert_eq!(texts.len(), 1, "{}", missing.report);
    assert!(
        texts[0].contains("`ghost_case` is not in `crates/one/src/checks/tests.rs` at base"),
        "{}",
        texts[0]
    );
}

#[test]
fn issue_442_ambiguous_suffix_is_refuted_only_when_every_reading_refutes() {
    let trace = run_line(NO_AREA, "is proven by `tests/shared.rs::ghost_case`.");
    let texts = finding_texts(&trace);
    assert_eq!(texts.len(), 1, "{}", trace.report);
    assert!(
        texts[0].contains(
            "`ghost_case` is not in `crates/one/tests/shared.rs` or `crates/two/tests/shared.rs` at base"
        ),
        "{}",
        texts[0]
    );
    // The readings disagree: one package has the symbol, so no refutation.
    assert_supported(&trace_of(NO_AREA, "`tests/shared.rs::shared_two`"));
    assert_supported(&trace_of(NO_AREA, "`tests/shared.rs::shared_one`"));
}

fn trace_of(declares: &str, span: &str) -> ClaimTrace {
    run_line(declares, &format!("is proven by {span}."))
}

#[test]
fn issue_442_the_tasks_own_package_decides_an_ambiguous_suffix() {
    // Package `one` is where the task works: package `two`'s symbol cannot
    // make the citation true.
    let trace = trace_of(IN_ONE, "`tests/shared.rs::shared_two`");
    let texts = finding_texts(&trace);
    assert_eq!(texts.len(), 1, "{}", trace.report);
    assert!(
        texts[0].contains("`shared_two` is not in `crates/one/tests/shared.rs` at base"),
        "{}",
        texts[0]
    );
    // The reading left out is named, so the author can cite it from the root.
    assert!(
        texts[0].contains("`crates/two/tests/shared.rs` also end(s) with `tests/shared.rs` but lie(s) outside the package(s) TASK-X-010 changes (`crates/one`)"),
        "{}",
        texts[0]
    );
    assert_supported(&trace_of(IN_ONE, "`tests/shared.rs::shared_one`"));
}

#[test]
fn issue_442_line_anchors_hold_only_inside_the_file() {
    // `crates/one/tests/shared.rs` has 5 lines.
    assert_supported(&trace_of(IN_ONE, "`crates/one/tests/shared.rs:2/5`"));
    assert_supported(&trace_of(IN_ONE, "`tests/shared.rs:1-5`"));
    for (span, written) in [
        ("`crates/one/tests/shared.rs:3-40`", "3-40"),
        ("`tests/shared.rs:291/319`", "291/319"),
    ] {
        let trace = trace_of(IN_ONE, span);
        let texts = finding_texts(&trace);
        assert_eq!(texts.len(), 1, "{span}: {}", trace.report);
        assert!(
            texts[0].contains(&format!(
                "line(s) {written} are not all lines of `crates/one/tests/shared.rs` (5 lines) at base"
            )),
            "{}",
            texts[0]
        );
    }
}

#[test]
fn issue_442_an_anchored_path_that_exists_nowhere_is_refuted_as_a_path() {
    let trace = trace_of(IN_ONE, "`crates/one/src/ghost.rs::build_record`");
    let texts = finding_texts(&trace);
    assert_eq!(texts.len(), 1, "{}", trace.report);
    assert!(
        texts[0].contains("`crates/one/src/ghost.rs`: `crates/one/src/ghost.rs` is not in the repository at base commit"),
        "{}",
        texts[0]
    );
    let trace = trace_of(IN_ONE, "`crates/one/src/ghost.rs:3`");
    assert_eq!(finding_texts(&trace).len(), 1, "{}", trace.report);
    assert!(
        finding_texts(&trace)[0]
            .contains("`crates/one/src/ghost.rs` is not in the repository at base"),
        "{}",
        trace.report
    );
}

#[test]
fn issue_442_a_placeholder_command_target_is_not_an_observable() {
    let trace = trace_of(
        IN_ONE,
        "`crates/one/src/lib.rs` with `cargo nextest … --test <name>`",
    );
    assert_supported(&trace);
    assert!(!trace.report.contains("<name>"), "{}", trace.report);
}

#[test]
fn issue_442_path_spans_split_into_path_and_anchor() {
    let runners = |first: &str| first == "cargo";
    let path = |span: &str| match observe::classify(span, &runners) {
        Some(Span::Path { raw, anchor, .. }) => (raw, anchor),
        other => panic!("{span}: {other:?}"),
    };
    assert_eq!(
        path("a/b.rs::c::d"),
        ("a/b.rs".into(), Some(Anchor::Symbol("d".into())))
    );
    let lines = |written: &str, numbers: &[u64]| {
        Some(Anchor::Lines {
            written: written.into(),
            numbers: numbers.to_vec(),
        })
    };
    assert_eq!(
        path("a/b.rs:291/319"),
        ("a/b.rs".into(), lines("291/319", &[291, 319]))
    );
    assert_eq!(path("a/b.rs:12:5"), ("a/b.rs".into(), lines("12", &[12])));
    assert_eq!(
        path("a/b.rs:202–212"),
        ("a/b.rs".into(), lines("202–212", &[202, 212]))
    );
    assert_eq!(
        path("a/b.rs:3,9,12"),
        ("a/b.rs".into(), lines("3,9,12", &[3, 9, 12]))
    );
    assert_eq!(
        path("./a/b.rs:3-9"),
        ("a/b.rs".into(), lines("3-9", &[3, 9]))
    );
    assert_eq!(path("a/b.rs:12-"), ("a/b.rs".into(), lines("12-", &[12])));
    assert_eq!(path("C:/a/b.rs:7"), ("C:/a/b.rs".into(), lines("7", &[7])));
    assert_eq!(path("a/b.rs"), ("a/b.rs".into(), None));
    // Not an anchor: the span stays whole, so its path decides.
    assert_eq!(path("a/b.rs:x"), ("a/b.rs:x".into(), None));
}

#[test]
fn issue_442_a_bare_name_that_is_a_cargo_target_is_supported() {
    // A test target is named by its file stem, which no file's text holds.
    assert_supported(&run_line(
        IN_ONE,
        "holds when `gate_refusals` passes, with ids from `crates/two/src/lib.rs:1`.",
    ));
    let trace = run_line(
        IN_ONE,
        "holds when `gate_refusal_ghost` passes, with ids from `crates/two/src/lib.rs:1`.",
    );
    let texts = finding_texts(&trace);
    assert_eq!(texts.len(), 1, "{}", trace.report);
    assert!(
        texts[0].contains("`gate_refusal_ghost` appears nowhere in the repository"),
        "{}",
        texts[0]
    );
}

#[test]
fn issue_442_a_symbol_placed_in_a_directory_is_looked_for_under_it() {
    assert_supported(&trace_of(
        IN_ONE,
        "`crates/one/src::stale_claim_is_an_error`",
    ));
    let trace = trace_of(IN_ONE, "`crates/one/tests::shared_two`");
    let texts = finding_texts(&trace);
    assert_eq!(texts.len(), 1, "{}", trace.report);
    assert!(
        texts[0].contains("`shared_two` is not in any file under `crates/one/tests` at base"),
        "{}",
        texts[0]
    );
}

#[test]
fn issue_442_an_anchor_in_a_file_a_task_declares_is_derived_not_refuted() {
    // The task declares `crates/one/src/lib.rs`, so the symbol may come.
    let trace = trace_of(IN_ONE, "`crates/one/src/lib.rs::not_yet_written`");
    assert_supported(&trace);
    assert!(
        trace.report.contains("1 derived from declarations")
            || trace.report.contains("2 derived from declarations"),
        "{}",
        trace.report
    );
}

#[test]
fn issue_442_a_directory_anchor_with_glob_characters_is_that_directory() {
    let trace = trace_of(IN_ONE, "`data/lit[1]::plain_item`");
    assert_supported(&trace);
    assert!(
        trace.report.contains("observables: 3 exist"),
        "the directory, its anchor and the declared file: {}",
        trace.report
    );
    let trace = trace_of(IN_ONE, "`data/lit[1]::only_in_sibling`");
    let texts = finding_texts(&trace);
    assert_eq!(texts.len(), 1, "{}", trace.report);
    assert!(
        texts[0].contains("`only_in_sibling` is not in any file under `data/lit[1]` at base"),
        "{}",
        texts[0]
    );
}
