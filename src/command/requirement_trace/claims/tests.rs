//! PLAN-2: the decomposition-time evidence index, built from a temp git
//! repository, and a false claim of each kind refuted against it.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::index::{EvidenceIndex, Resolved};
use super::{ClaimTrace, falsify_claims, observe};

/// A committed repository: a package `demo` with a library, one symbol in
/// each of two files, and an integration test target `existing`.
pub(super) fn repository(root: &Path) -> String {
    let files = [
        (
            "Cargo.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        ),
        ("src/lib.rs", "pub mod other;\npub fn present_symbol() {}\n"),
        ("src/other.rs", "pub fn other_thing() {}\n"),
        ("tests/existing.rs", "#[test]\nfn works() {}\n"),
    ];
    for (path, text) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "base"]);
    git(&["rev-parse", "HEAD"])
}

pub(super) fn record(
    root: &Path,
    base: &str,
) -> archon_workflow::repository_record::RepositoryRecordV1 {
    archon_workflow::repository_record::RepositoryRecordV1 {
        schema_version: archon_workflow::repository_record::REPOSITORY_RECORD_SCHEMA_VERSION,
        repository_root: root.display().to_string(),
        base_commit: base.to_string(),
        decomposition_run_id: "run".into(),
        recorded_at: "now".into(),
    }
}

/// A task set recorded against `root`, one TASK file per `(id, body)`.
pub(super) fn task_set(dir: &Path, root: &Path, base: &str, tasks: &[(&str, &str)]) -> PathBuf {
    let task_dir = dir.join("tasks");
    std::fs::create_dir_all(&task_dir).unwrap();
    archon_workflow::repository_record::write_repository_record(&task_dir, &record(root, base))
        .unwrap();
    for (id, body) in tasks {
        std::fs::write(task_dir.join(format!("{id}.md")), body).unwrap();
    }
    task_dir
}

pub(super) fn task(id: &str, implements: &str, body: &str) -> String {
    task_with(id, implements, "[]", body)
}

/// A task file the task-universe reader accepts, with `deliverables` as its
/// `deliverable_contracts` value.
pub(super) fn task_with(id: &str, implements: &str, deliverables: &str, body: &str) -> String {
    format!(
        "```yaml\ntask_id: {id}\ntitle: t\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [{implements}]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: {deliverables}\n```\n\n{body}"
    )
}

fn run(tasks: &[(&str, &str)]) -> ClaimTrace {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let base = repository(&root);
    let task_dir = task_set(dir.path(), &root, &base, tasks);
    let (bindings, findings) =
        crate::command::requirement_trace::load_bindings_with_findings(&task_dir).unwrap();
    assert!(findings.is_empty(), "{findings:?}");
    falsify_claims(&task_dir, bindings).expect("falsify")
}

const DECLARES_LIB: &str = "## Files Expected to Change\n\n- `src/lib.rs` — exists (2 lines)\n\n";

#[test]
fn the_index_is_built_from_the_recorded_repository_at_its_base_commit() {
    let dir = tempfile::tempdir().unwrap();
    let base = repository(dir.path());
    let mut index = EvidenceIndex::load(&record(dir.path(), &base)).unwrap();
    index.declare("src/new.rs", "TASK-X-010");
    assert_eq!(
        index.resolve("src/lib.rs"),
        Resolved::Exists("src/lib.rs".into())
    );
    assert_eq!(
        index.resolve("tests/existing.rs"),
        Resolved::Exists("tests/existing.rs".into())
    );
    assert!(
        matches!(index.resolve("src/new.rs"), Resolved::Derived(_, owners) if owners.contains("TASK-X-010"))
    );
    assert_eq!(
        index.resolve("src/ghost.rs"),
        Resolved::Missing("src/ghost.rs".into())
    );
    assert!(index.contains_word("src/lib.rs", "present_symbol"));
    assert!(!index.contains_word("src/lib.rs", "other_thing"));
    assert_eq!(index.package_dir("demo").as_deref(), Some(""));
    assert_eq!(index.package_dir("ghost"), None);
    assert_eq!(
        index
            .relative(&dir.path().join("src/lib.rs").display().to_string())
            .as_deref(),
        Some("src/lib.rs")
    );
    assert_eq!(index.relative("/elsewhere/x.rs"), None);
}

#[test]
fn a_claim_naming_a_file_no_task_creates_is_refuted_on_its_own_body() {
    let body = task(
        "TASK-X-010",
        "REQ-X-001",
        &format!(
            "{DECLARES_LIB}## Acceptance Criteria\n\n- REQ-X-001 is proven by `src/missing.rs`.\n"
        ),
    );
    let trace = run(&[("TASK-X-010", &body)]);
    assert_eq!(trace.findings.len(), 1, "{}", trace.report);
    let finding = &trace.findings[0];
    assert!(
        finding
            .text
            .contains("task TASK-X-010: its claim of 'REQ-X-001' is refuted"),
        "{}",
        finding.text
    );
    assert!(
        finding.text.contains("`src/missing.rs`"),
        "{}",
        finding.text
    );
    assert_eq!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::Body
    );
    assert!(
        finding.source_path.ends_with("TASK-X-010.md"),
        "{:?}",
        finding.source_path
    );
    assert!(
        trace
            .report
            .contains("1 claim(s): 1 tested, 1 refuted, 0 untestable"),
        "{}",
        trace.report
    );
}

#[test]
fn a_symbol_a_file_the_task_may_not_change_does_not_contain_is_refuted() {
    let body = task(
        "TASK-X-010",
        "REQ-X-001",
        &format!(
            "{DECLARES_LIB}## Scope\n\n- REQ-X-001 consumes `other_thing`, `present_symbol` and `ghost_fn` from `src/other.rs`.\n"
        ),
    );
    let trace = run(&[("TASK-X-010", &body)]);
    let texts: Vec<&str> = trace.findings.iter().map(|f| f.text.as_str()).collect();
    assert_eq!(texts.len(), 1, "{}", trace.report);
    assert!(
        texts[0].contains("`ghost_fn` appears nowhere in the repository")
            && texts[0].contains("places it in `src/other.rs`"),
        "{}",
        texts[0]
    );
    assert!(
        !texts[0].contains("other_thing") && !texts[0].contains("present_symbol"),
        "an existing symbol is not refuted: {}",
        texts[0]
    );
}

#[test]
fn a_claim_no_declared_file_can_serve_is_refuted() {
    let body = task(
        "TASK-X-010",
        "REQ-X-001",
        "## Scope\n\n- REQ-X-001 is delivered by `present_symbol`.\n",
    );
    let trace = run(&[("TASK-X-010", &body)]);
    assert!(
        trace
            .findings
            .iter()
            .any(|f| f.text.contains("declares no file it may change")),
        "{}",
        trace.report
    );
}

#[test]
fn a_verifier_naming_a_test_target_nothing_creates_is_refuted() {
    let body = task(
        "TASK-X-010",
        "REQ-X-001",
        &format!(
            "{DECLARES_LIB}## Scope\n\n- REQ-X-001 lives in `present_symbol`.\n\n## Focused Tests\n\n- `cargo test -p demo --test existing`\n- `cargo test -p demo --test ghost`\n"
        ),
    );
    let trace = run(&[("TASK-X-010", &body)]);
    assert_eq!(trace.findings.len(), 1, "{}", trace.report);
    assert!(
        trace.findings[0].text.contains("test target `ghost`"),
        "{}",
        trace.findings[0].text
    );
    assert!(
        trace.findings[0].text.contains("`tests/ghost.rs`"),
        "{}",
        trace.findings[0].text
    );
}

#[test]
fn a_claim_the_body_never_names_is_an_open_untestable_finding_with_its_reason() {
    let body = task(
        "TASK-X-010",
        "REQ-X-001, REQ-X-002",
        &format!("{DECLARES_LIB}## Scope\n\n- REQ-X-001 lives in `present_symbol`.\n"),
    );
    let trace = run(&[("TASK-X-010", &body)]);
    assert_eq!(trace.findings.len(), 1, "{}", trace.report);
    let text = &trace.findings[0].text;
    assert!(
        text.contains("its claim of 'REQ-X-002' cannot be tested"),
        "{text}"
    );
    assert!(
        text.contains("never names REQ-X-002 outside its implements list"),
        "{text}"
    );
    assert!(
        trace.report.contains("UNTESTABLE TASK-X-010 REQ-X-002"),
        "{}",
        trace.report
    );
}

#[test]
fn a_grounded_set_tests_every_claim_including_files_another_task_creates() {
    // Declared only as a deliverable contract, not under Files Expected.
    let creator = task_with(
        "TASK-X-010",
        "REQ-X-001",
        r#"[{"kind":"rust-source","artifact_path":"src/new.rs"}]"#,
        "## Scope\n\n- REQ-X-001: `new_thing` in `src/new.rs`.\n",
    );
    let consumer = task(
        "TASK-X-020",
        "REQ-X-002",
        &format!(
            "{DECLARES_LIB}## Scope\n\n- REQ-X-002 calls `new_thing` from `src/new.rs` and `present_symbol`.\n\n## Focused Tests\n\n- `cargo test -p demo --test existing`\n- `test \"$(wc -l < src/lib.rs)\" -lt 5`\n"
        ),
    );
    let trace = run(&[("TASK-X-010", &creator), ("TASK-X-020", &consumer)]);
    assert!(trace.findings.is_empty(), "{}", trace.report);
    assert!(
        trace
            .report
            .contains("2 claim(s): 2 tested, 0 refuted, 0 untestable"),
        "{}",
        trace.report
    );
    assert!(
        trace.report.contains("tested TASK-X-020 REQ-X-002"),
        "{}",
        trace.report
    );
}

#[test]
fn a_task_set_without_a_repository_record_is_not_silently_passed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("TASK-X-010.md"),
        task("TASK-X-010", "REQ-X-001", "- REQ-X-001\n"),
    )
    .unwrap();
    let (bindings, _) =
        crate::command::requirement_trace::load_bindings_with_findings(dir.path()).unwrap();
    let error = falsify_claims(dir.path(), bindings)
        .err()
        .expect("no record is an error");
    assert!(error.to_string().contains("repository.lock"), "{error}");
}

#[test]
fn claims_are_found_inside_written_ranges_and_lists() {
    let line = "- Row rules (REQ-A-050…053, 090…098) and REQ-A-100/101/102; AC-A-005 / DONE-8.";
    for id in [
        "REQ-A-050",
        "REQ-A-052",
        "REQ-A-090",
        "REQ-A-095",
        "REQ-A-098",
        "REQ-A-101",
        "AC-A-005",
        "DONE-8",
    ] {
        assert!(observe::mentions(line, id), "{id}");
    }
    for id in [
        "REQ-A-054",
        "REQ-A-060",
        "REQ-A-099",
        "REQ-A-103",
        "REQ-B-050",
        "DONE-9",
        "REQ-A-5",
    ] {
        assert!(!observe::mentions(line, id), "{id}");
    }
    assert!(observe::mentions("REQ-A-001..003", "REQ-A-002"));
    assert!(!observe::mentions("REQ-A-0010", "REQ-A-001"));
}

#[test]
fn spans_are_classified_without_guessing() {
    let runners = |first: &str| first == "cargo";
    let classify = |span: &str| observe::classify(span, &runners);
    assert!(
        matches!(classify("src/a.rs:12"), Some(observe::Span::Path { raw, directory: false }) if raw == "src/a.rs")
    );
    assert!(matches!(
        classify("out/dir/"),
        Some(observe::Span::Path {
            directory: true,
            ..
        })
    ));
    assert_eq!(
        classify("rollup(checks)"),
        Some(observe::Span::Symbol("rollup".into()))
    );
    assert_eq!(
        classify("DatasetMetadata"),
        Some(observe::Span::Symbol("DatasetMetadata".into()))
    );
    assert_eq!(
        classify("a::b::Thing"),
        Some(observe::Span::Symbol("Thing".into()))
    );
    assert_eq!(
        classify("snake_case_fn"),
        Some(observe::Span::Symbol("snake_case_fn".into()))
    );
    for prose in [
        "passed",
        "mcp__prefix__",
        "status=passed",
        "high < low",
        "v1.json",
    ] {
        assert_eq!(classify(prose), None, "{prose}");
    }
    assert_eq!(
        classify("cargo test -p a"),
        Some(observe::Span::Command("cargo test -p a".into()))
    );
}

/// Read-only prediction on a recorded task set: `ARCHON_CLAIM_TRACE_TASKS`
/// names its directory. Prints the report and every finding.
#[test]
#[ignore = "reads the task set named by ARCHON_CLAIM_TRACE_TASKS"]
fn claim_trace_of_a_recorded_task_set() {
    let Some(dir) = std::env::var_os("ARCHON_CLAIM_TRACE_TASKS") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let (bindings, findings) =
        crate::command::requirement_trace::load_bindings_with_findings(&dir).unwrap();
    assert!(findings.is_empty(), "{findings:?}");
    let started = std::time::Instant::now();
    let trace = falsify_claims(&dir, bindings).expect("falsify");
    println!("{}", trace.report);
    for finding in &trace.findings {
        println!("FINDING {}", finding.text);
    }
    println!("elapsed {:?}", started.elapsed());
}
