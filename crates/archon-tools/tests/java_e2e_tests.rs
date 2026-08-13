//! End-to-end runs against the Gradle and Maven fixture projects (#176).
//!
//! These drive real builds. They are opt-in via `ARCHON_JAVA_E2E=1` because a
//! machine without a JDK cannot run them and a cold run downloads the analysis
//! plugins — but when the variable IS set and the toolchain is missing, they
//! fail rather than skip. A test that quietly passes when it did not run is
//! worse than no test: it makes a broken toolchain look healthy.
//!
//! Everything these assert about report formats is also covered by
//! `java_report_tests.rs` against checked-in report shapes, so the parsing
//! contract is still tested on machines that never run this file.

use std::path::{Path, PathBuf};

use archon_tools::java::JavaToolchain;
use archon_tools::java::finding::Source;
use archon_tools::java::project::{self, BuildSystem, Stage};
use archon_tools::java::reports;
use archon_tools::tool::{Tool, ToolContext};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/java")
        .join(name)
}

/// Whether to run. Returns false to skip, panics when the run was demanded but
/// cannot happen.
fn e2e_enabled(required: &str) -> bool {
    if std::env::var("ARCHON_JAVA_E2E").as_deref() != Ok("1") {
        eprintln!("skipping: set ARCHON_JAVA_E2E=1 to run the Java end-to-end tests");
        return false;
    }
    assert!(
        which::which(required).is_ok(),
        "ARCHON_JAVA_E2E=1 was set but `{required}` is not on PATH. \
         Install the toolchain with scripts/install-system-deps.ps1 -WithJava \
         (or --with-java on POSIX), or unset ARCHON_JAVA_E2E."
    );
    true
}

async fn run(root: &Path, operation: &str) -> String {
    let ctx = ToolContext {
        working_dir: root.to_path_buf(),
        ..Default::default()
    };
    let result = JavaToolchain
        .execute(
            serde_json::json!({
                "operation": operation,
                "path": root.to_string_lossy(),
                "limit": 20,
            }),
            &ctx,
        )
        .await;
    assert!(!result.is_error, "{operation} failed:\n{}", result.content);
    result.content
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[test]
fn the_fixture_projects_are_detected_as_what_they_are() {
    let gradle = project::detect(&fixture("gradle-sample")).expect("gradle-sample detected");
    assert_eq!(gradle.build_system, BuildSystem::Gradle);

    let maven = project::detect(&fixture("maven-sample")).expect("maven-sample detected");
    assert_eq!(maven.build_system, BuildSystem::Maven);
}

// ---------------------------------------------------------------------------
// Gradle
// ---------------------------------------------------------------------------

/// The deliberately defective fixture, driven through a real Gradle build.
///
/// Each assertion names a defect the TOOLS found, not one a model described:
/// an injection sink, a swallowed exception, and a nine-parameter method.
#[tokio::test]
async fn gradle_analysis_names_every_planted_defect() {
    if !e2e_enabled("gradle") {
        return;
    }
    let root = fixture("gradle-sample");
    run(&root, "compile").await;
    run(&root, "analyze").await;

    let findings = reports::collect(&root, BuildSystem::Gradle, Stage::Analyze);
    assert!(!findings.is_empty(), "analysis produced no findings at all");

    for rule in ["EmptyCatchBlock", "ParameterNumber", "SQL_INJECTION_JDBC"] {
        assert!(
            findings.iter().any(|f| f.rule == rule),
            "{rule} was not reported. Findings: {:#?}",
            findings.iter().map(|f| &f.rule).collect::<Vec<_>>()
        );
    }
}

/// The security finding has to arrive with its CWE attached — that is what
/// turns "SpotBugs is unhappy" into a weakness on the OWASP and SANS lists.
#[tokio::test]
async fn gradle_security_finding_carries_a_cwe() {
    if !e2e_enabled("gradle") {
        return;
    }
    let root = fixture("gradle-sample");
    run(&root, "compile").await;
    run(&root, "analyze").await;

    let findings = reports::collect(&root, BuildSystem::Gradle, Stage::Analyze);
    let injection = findings
        .iter()
        .find(|f| f.rule == "SQL_INJECTION_JDBC")
        .expect("the injection sink was not reported");
    assert_eq!(injection.cwe.as_deref(), Some("CWE-89"));
    assert_eq!(injection.source, Source::SpotBugs);
    assert!(injection.line.is_some(), "the finding was not located");
}

#[tokio::test]
async fn gradle_test_failures_are_read_back() {
    if !e2e_enabled("gradle") {
        return;
    }
    let root = fixture("gradle-sample");
    run(&root, "test").await;

    let findings = reports::collect(&root, BuildSystem::Gradle, Stage::Test);
    assert_eq!(
        findings.len(),
        1,
        "expected exactly the one deliberately failing test: {findings:#?}"
    );
    assert!(
        findings[0]
            .message
            .contains("deliberatelyFailsSoTheReportHasSomethingToParse"),
        "got: {}",
        findings[0].message
    );
}

// ---------------------------------------------------------------------------
// Maven
// ---------------------------------------------------------------------------

/// The same source, the same rules, the other build system. A difference in
/// what is found here is a difference between Gradle and Maven, because the
/// inputs are byte-identical.
#[tokio::test]
async fn maven_analysis_names_every_planted_defect() {
    if !e2e_enabled("mvn") {
        return;
    }
    let root = fixture("maven-sample");
    run(&root, "compile").await;
    run(&root, "analyze").await;

    let findings = reports::collect(&root, BuildSystem::Maven, Stage::Analyze);
    assert!(!findings.is_empty(), "analysis produced no findings at all");

    for rule in ["EmptyCatchBlock", "ParameterNumber", "SQL_INJECTION_JDBC"] {
        assert!(
            findings.iter().any(|f| f.rule == rule),
            "{rule} was not reported. Findings: {:#?}",
            findings.iter().map(|f| &f.rule).collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn maven_test_failures_are_read_back() {
    if !e2e_enabled("mvn") {
        return;
    }
    let root = fixture("maven-sample");
    run(&root, "test").await;

    let findings = reports::collect(&root, BuildSystem::Maven, Stage::Test);
    assert_eq!(findings.len(), 1, "got: {findings:#?}");
    assert_eq!(findings[0].source, Source::Tests);
}

// ---------------------------------------------------------------------------
// The rendered surface
// ---------------------------------------------------------------------------

/// What the model actually receives: a severity-ordered batch with the security
/// finding at the top, not a report dump.
#[tokio::test]
async fn the_rendered_batch_leads_with_the_security_finding() {
    if !e2e_enabled("gradle") {
        return;
    }
    let root = fixture("gradle-sample");
    run(&root, "compile").await;
    let rendered = run(&root, "analyze").await;

    let injection_at = rendered
        .find("SQL_INJECTION_JDBC")
        .unwrap_or_else(|| panic!("injection missing from the batch:\n{rendered}"));
    let style_at = rendered
        .find("FileLength")
        .unwrap_or_else(|| panic!("style finding missing from the batch:\n{rendered}"));
    assert!(
        injection_at < style_at,
        "the injection must be presented before the style violation:\n{rendered}"
    );
    assert!(rendered.contains("CWE-89"), "got:\n{rendered}");
}
