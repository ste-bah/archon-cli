//! Build-system detection, stage arguments and issue selection (#176).

use std::fs;
use std::path::Path;

use archon_tools::java::finding::{Finding, Severity, Source, select_issues, severity_counts};
use archon_tools::java::project::{self, BuildSystem, Launcher, Stage, stage_args};

fn touch(root: &Path, name: &str) {
    fs::write(root.join(name), "").expect("write marker");
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[test]
fn settings_gradle_identifies_a_gradle_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    touch(temp.path(), "settings.gradle.kts");
    let project = project::detect(temp.path()).expect("detected");
    assert_eq!(project.build_system, BuildSystem::Gradle);
}

#[test]
fn pom_identifies_a_maven_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    touch(temp.path(), "pom.xml");
    let project = project::detect(temp.path()).expect("detected");
    assert_eq!(project.build_system, BuildSystem::Maven);
}

/// A Gradle build that also carries a `pom.xml` — for publishing metadata, or
/// left over from a migration — is common; the reverse is not. Gradle wins.
#[test]
fn gradle_wins_when_both_build_files_are_present() {
    let temp = tempfile::tempdir().expect("tempdir");
    touch(temp.path(), "build.gradle");
    touch(temp.path(), "pom.xml");
    let project = project::detect(temp.path()).expect("detected");
    assert_eq!(project.build_system, BuildSystem::Gradle);
}

/// Not every directory handed to this is a Java project, and saying so is a
/// real answer rather than a failure.
#[test]
fn a_directory_with_no_build_file_is_not_a_java_project() {
    let temp = tempfile::tempdir().expect("tempdir");
    touch(temp.path(), "Cargo.toml");
    assert!(project::detect(temp.path()).is_none());
}

// ---------------------------------------------------------------------------
// Launcher
// ---------------------------------------------------------------------------

/// The wrapper pins the build-tool version the project was written against, so
/// preferring it is the difference between reproducing the project's build and
/// running a different one.
#[test]
fn the_gradle_wrapper_is_preferred_over_gradle_on_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    touch(temp.path(), "build.gradle");
    let wrapper = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    };
    touch(temp.path(), wrapper);

    let project = project::detect(temp.path()).expect("detected");
    match project.launcher {
        Launcher::Wrapper(path) => assert!(path.ends_with(wrapper), "got {}", path.display()),
        other => panic!("expected the wrapper, got {other:?}"),
    }
}

#[test]
fn the_maven_wrapper_is_preferred_over_mvn_on_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    touch(temp.path(), "pom.xml");
    let wrapper = if cfg!(windows) { "mvnw.cmd" } else { "mvnw" };
    touch(temp.path(), wrapper);

    let project = project::detect(temp.path()).expect("detected");
    assert!(matches!(project.launcher, Launcher::Wrapper(_)));
}

/// Without a wrapper there is still something to run — it is just not
/// guaranteed to be the version the project expects, which is why `detect`
/// reports which of the two it chose.
#[test]
fn a_project_without_a_wrapper_falls_back_to_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    touch(temp.path(), "build.gradle");
    let project = project::detect(temp.path()).expect("detected");
    assert!(matches!(project.launcher, Launcher::OnPath(_)));
}

// ---------------------------------------------------------------------------
// Stage arguments
// ---------------------------------------------------------------------------

/// A run that stops at the first violating task never reaches SpotBugs, and the
/// point of the analysis pass is to collect everything the tools can see in one
/// go.
#[test]
fn gradle_analysis_keeps_going_past_the_first_violation() {
    let args = stage_args(BuildSystem::Gradle, Stage::Analyze);
    assert!(args.contains(&"--continue"), "got: {args:?}");
    for task in ["checkstyleMain", "pmdMain", "spotbugsMain"] {
        assert!(args.contains(&task), "{task} missing from {args:?}");
    }
}

#[test]
fn maven_analysis_runs_all_three_analysers() {
    let args = stage_args(BuildSystem::Maven, Stage::Analyze);
    for goal in ["checkstyle:checkstyle", "pmd:pmd", "spotbugs:spotbugs"] {
        assert!(args.contains(&goal), "{goal} missing from {args:?}");
    }
}

/// Surefire aborts the build on the first failing test by default, which would
/// leave the remaining test reports unwritten and the failure list incomplete.
#[test]
fn maven_tests_do_not_abort_on_the_first_failure() {
    let args = stage_args(BuildSystem::Maven, Stage::Test);
    assert!(
        args.contains(&"-Dmaven.test.failure.ignore=true"),
        "got: {args:?}"
    );
}

/// SpotBugs reads bytecode, not source, so the compile stage has to produce
/// test classes as well as main ones before analysis can see them.
#[test]
fn compile_builds_test_sources_too() {
    assert!(stage_args(BuildSystem::Gradle, Stage::Compile).contains(&"testClasses"));
    assert!(stage_args(BuildSystem::Maven, Stage::Compile).contains(&"test-compile"));
}

/// Every Gradle invocation, without exception.
///
/// A surviving Gradle daemon inherits the build's handles and outlives it — and
/// on Windows a child inherits every inheritable handle in the spawning process,
/// not just its redirected stdio. So a daemon left running holds open a copy of
/// whatever pipe archon's output goes to, and the caller reading that pipe hangs
/// long after the build finished. This is not a tuning knob.
#[test]
fn every_gradle_stage_refuses_to_leave_a_daemon_running() {
    for stage in [Stage::Compile, Stage::Analyze, Stage::Test] {
        let args = stage_args(BuildSystem::Gradle, stage);
        assert!(
            args.contains(&"--no-daemon"),
            "{stage:?} would leave a daemon holding the caller's pipe: {args:?}"
        );
        assert!(
            args.contains(&"--console=plain"),
            "{stage:?} would write an ANSI progress bar into a captured pipe: {args:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Issue selection
// ---------------------------------------------------------------------------

fn finding(rule: &str, severity: Severity, line: u32) -> Finding {
    Finding {
        file: "Foo.java".to_string(),
        line: Some(line),
        severity,
        rule: rule.to_string(),
        cwe: None,
        message: format!("{rule} message"),
        source: Source::Pmd,
    }
}

/// Presenting a whole report at once measurably degrades fix quality, so a pass
/// gets a small batch — and it has to be the most serious findings, not the
/// first ones the parser happened to emit.
#[test]
fn selection_takes_the_most_severe_first() {
    let findings = vec![
        finding("Minor", Severity::Minor, 3),
        finding("Blocker", Severity::Blocker, 1),
        finding("Major", Severity::Major, 2),
        finding("Critical", Severity::Critical, 4),
    ];
    let selected = select_issues(&findings, 2);
    assert_eq!(selected.len(), 2);
    assert_eq!(selected[0].rule, "Blocker");
    assert_eq!(selected[1].rule, "Critical");
}

/// Two findings of equal severity are ordered by where they are, so a batch
/// does not shuffle between runs over unchanged code.
#[test]
fn selection_is_stable_within_a_severity() {
    let findings = vec![
        finding("Second", Severity::Major, 90),
        finding("First", Severity::Major, 10),
    ];
    let selected = select_issues(&findings, 2);
    assert_eq!(selected[0].line, Some(10));
    assert_eq!(selected[1].line, Some(90));
}

#[test]
fn selection_returns_everything_when_under_the_limit() {
    let findings = vec![finding("Only", Severity::Major, 1)];
    assert_eq!(select_issues(&findings, 5).len(), 1);
}

#[test]
fn severity_counts_are_ordered_most_serious_first() {
    let findings = vec![
        finding("a", Severity::Minor, 1),
        finding("b", Severity::Blocker, 2),
        finding("c", Severity::Minor, 3),
    ];
    let counts = severity_counts(&findings);
    assert_eq!(counts, vec![(Severity::Blocker, 1), (Severity::Minor, 2)]);
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// A finding is injected at its line, severity-weighted — presenting the whole
/// report unanchored is what degrades the result.
#[test]
fn a_finding_renders_with_its_location_and_rule() {
    let rendered = finding("EmptyCatchBlock", Severity::Major, 51).render();
    assert!(rendered.starts_with("Foo.java:51"), "got: {rendered}");
    assert!(rendered.contains("EmptyCatchBlock"), "got: {rendered}");
    assert!(rendered.contains("major"), "got: {rendered}");
}

#[test]
fn a_security_finding_renders_its_cwe() {
    let mut f = finding("SQL_INJECTION_JDBC", Severity::Blocker, 61);
    f.cwe = Some("CWE-89".to_string());
    f.source = Source::SpotBugs;
    let rendered = f.render();
    assert!(rendered.contains("CWE-89"), "got: {rendered}");
}
