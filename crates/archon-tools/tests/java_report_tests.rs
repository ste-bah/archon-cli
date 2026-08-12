//! Parsing the JVM analysis tools' report files (#176).
//!
//! The fixtures below are the shapes the real tools emit, trimmed to the
//! attributes the parsers read. They exist because the console output of these
//! tools carries neither the rule identity nor the CWE mapping, so the report
//! files are the only source that makes a finding actionable — and a parser
//! that silently returns nothing would make a broken analysis stage look clean.

use std::fs;

use archon_tools::java::finding::{Severity, Source};
use archon_tools::java::parse;
use archon_tools::java::project::{BuildSystem, Stage};
use archon_tools::java::reports;

const CHECKSTYLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<checkstyle version="10.17.0">
<file name="/src/main/java/com/example/OrderService.java">
<error line="42" column="9" severity="warning" message="Empty statement." source="com.puppycrawl.tools.checkstyle.checks.coding.EmptyStatementCheck"/>
<error line="88" column="5" severity="error" message="Method length is 210 lines (max allowed is 60)." source="com.puppycrawl.tools.checkstyle.checks.sizes.MethodLengthCheck"/>
</file>
</checkstyle>
"#;

const PMD_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<pmd version="7.4.0">
<file name="/src/main/java/com/example/OrderService.java">
<violation beginline="51" endline="53" rule="EmptyCatchBlock" ruleset="Best Practices" priority="3">
Avoid empty catch blocks
</violation>
<violation beginline="12" endline="12" rule="ExcessiveParameterList" ruleset="Design" priority="1">
Avoid long parameter lists.
</violation>
</file>
</pmd>
"#;

/// A FindSecBugs finding: the `cweid` attribute is what turns "SpotBugs is
/// unhappy" into a named weakness, and is the reason this pipeline reads the
/// XML rather than the console.
///
/// The attribute values here are copied from a real run of the Maven fixture
/// project, not invented. That matters for `rank`: a live SQL injection comes
/// back at rank 12, not the single-digit rank one would guess for the most
/// serious thing in the file.
const SPOTBUGS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<BugCollection version="4.8.6">
<BugInstance instanceHash="8182a186171da3bbc166caef332bfb28" cweid="89" rank="12" abbrev="SECSQLIJDBC" category="SECURITY" priority="2" type="SQL_INJECTION_JDBC">
<ShortMessage>Nonconstant string passed to execute or addBatch method on an SQL statement</ShortMessage>
<Class classname="com.example.OrderService"/>
<Method classname="com.example.OrderService" name="lookup"/>
<SourceLine classname="com.example.OrderService" start="61" end="61" sourcefile="OrderService.java" sourcepath="com/example/OrderService.java"/>
</BugInstance>
<BugInstance type="DM_DEFAULT_ENCODING" priority="2" rank="17" abbrev="Dm" category="I18N">
<ShortMessage>Reliance on default encoding</ShortMessage>
<Class classname="com.example.OrderService"/>
<SourceLine classname="com.example.OrderService" start="70" end="70" sourcefile="OrderService.java" sourcepath="com/example/OrderService.java"/>
</BugInstance>
</BugCollection>
"#;

const JUNIT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="com.example.OrderServiceTest" tests="3" failures="1" errors="1" skipped="0">
<testcase name="pricesAreRounded" classname="com.example.OrderServiceTest" time="0.004"/>
<testcase name="rejectsNegativeQuantity" classname="com.example.OrderServiceTest" time="0.002">
<failure message="expected:&lt;400&gt; but was:&lt;200&gt;" type="java.lang.AssertionError">stack trace here</failure>
</testcase>
<testcase name="loadsConfig" classname="com.example.OrderServiceTest" time="0.001">
<error message="config.properties not found" type="java.io.FileNotFoundException">stack trace here</error>
</testcase>
</testsuite>
"#;

// ---------------------------------------------------------------------------
// Checkstyle
// ---------------------------------------------------------------------------

#[test]
fn checkstyle_findings_are_located_and_named() {
    let findings = parse::checkstyle(CHECKSTYLE_XML);
    assert_eq!(findings.len(), 2, "got: {findings:?}");

    let empty = &findings[0];
    assert_eq!(empty.file, "/src/main/java/com/example/OrderService.java");
    assert_eq!(empty.line, Some(42));
    assert_eq!(empty.source, Source::Checkstyle);
    assert_eq!(empty.severity, Severity::Major, "warning maps to major");
}

/// The `source` attribute is a fully qualified class name; only the rule at the
/// end of it is what a reader looks up.
#[test]
fn checkstyle_rule_is_the_bare_check_name() {
    let findings = parse::checkstyle(CHECKSTYLE_XML);
    assert_eq!(findings[0].rule, "EmptyStatement");
    assert_eq!(findings[1].rule, "MethodLength");
}

#[test]
fn checkstyle_error_severity_outranks_warning() {
    let findings = parse::checkstyle(CHECKSTYLE_XML);
    assert_eq!(findings[1].severity, Severity::Critical);
}

// ---------------------------------------------------------------------------
// PMD
// ---------------------------------------------------------------------------

/// PMD is the only one of these tools that puts its message in element text
/// rather than an attribute, so an attribute-only parser silently yields
/// findings with empty messages.
#[test]
fn pmd_message_comes_from_element_text() {
    let findings = parse::pmd(PMD_XML);
    assert_eq!(findings.len(), 2, "got: {findings:?}");
    assert_eq!(findings[0].message, "Avoid empty catch blocks");
    assert_eq!(findings[0].rule, "EmptyCatchBlock");
    assert_eq!(findings[0].line, Some(51));
}

#[test]
fn pmd_priority_one_is_a_blocker() {
    let findings = parse::pmd(PMD_XML);
    assert_eq!(findings[1].rule, "ExcessiveParameterList");
    assert_eq!(findings[1].severity, Severity::Blocker);
}

// ---------------------------------------------------------------------------
// SpotBugs / FindSecBugs
// ---------------------------------------------------------------------------

#[test]
fn spotbugs_security_finding_carries_its_cwe() {
    let findings = parse::spotbugs(SPOTBUGS_XML);
    assert_eq!(findings.len(), 2, "got: {findings:?}");

    let injection = &findings[0];
    assert_eq!(injection.rule, "SQL_INJECTION_JDBC");
    assert_eq!(injection.cwe.as_deref(), Some("CWE-89"));
    assert_eq!(injection.file, "com/example/OrderService.java");
    assert_eq!(injection.line, Some(61));
}

/// SpotBugs ranks by how likely a report is to be a real defect, not by what it
/// costs when it is — so a live SQL injection arrives at rank 12, which on the
/// rank scale alone lands in the same band as an unused local. Reporting an
/// injection below a style violation would invert exactly the priority this
/// pipeline exists to establish.
#[test]
fn a_security_finding_is_not_ranked_below_a_style_violation() {
    let findings = parse::spotbugs(SPOTBUGS_XML);
    let injection = &findings[0];
    assert_eq!(
        injection.severity,
        Severity::Critical,
        "rank 12 alone would make this major; the SECURITY category floors it"
    );

    let style = &parse::checkstyle(CHECKSTYLE_XML)[1];
    assert!(
        injection.severity <= style.severity,
        "a CWE-89 injection ({}) must not sort below a Checkstyle finding ({})",
        injection.severity,
        style.severity
    );
}

/// Only the security patterns carry a CWE; a style or i18n finding has none,
/// and inventing one would be worse than reporting its absence.
#[test]
fn spotbugs_non_security_finding_has_no_cwe() {
    let findings = parse::spotbugs(SPOTBUGS_XML);
    assert_eq!(findings[1].rule, "DM_DEFAULT_ENCODING");
    assert_eq!(findings[1].cwe, None);
    assert_eq!(findings[1].severity, Severity::Minor, "rank 17");
}

// ---------------------------------------------------------------------------
// JUnit / Surefire
// ---------------------------------------------------------------------------

/// A passing test is not a finding. Emitting one per test would bury the
/// failures under the successes in every batch.
#[test]
fn junit_reports_only_failures_and_errors() {
    let findings = parse::junit(JUNIT_XML);
    assert_eq!(findings.len(), 2, "got: {findings:?}");
    assert!(findings.iter().all(|f| f.source == Source::Tests));
    assert!(findings.iter().all(|f| f.severity == Severity::Blocker));
}

#[test]
fn junit_failure_message_names_the_test() {
    let findings = parse::junit(JUNIT_XML);
    assert!(
        findings[0].message.starts_with("rejectsNegativeQuantity:"),
        "got: {}",
        findings[0].message
    );
    assert!(
        findings[0].message.contains("expected:<400> but was:<200>"),
        "XML entities should be unescaped, got: {}",
        findings[0].message
    );
    assert_eq!(findings[1].rule, "test-error");
}

// ---------------------------------------------------------------------------
// javac
// ---------------------------------------------------------------------------

/// Compile is the one stage with no report file: neither Gradle nor Maven
/// persists javac's diagnostics, so they are parsed from captured output.
#[test]
fn javac_errors_are_located() {
    let output = "\
> Task :compileJava FAILED
/work/src/main/java/com/example/OrderService.java:17: error: cannot find symbol
        Statment s = null;
        ^
/work/src/main/java/com/example/OrderService.java:22: warning: [deprecation] foo() is deprecated
1 error
";
    let findings = parse::javac(output);
    assert_eq!(findings.len(), 2, "got: {findings:?}");
    assert_eq!(
        findings[0].file,
        "/work/src/main/java/com/example/OrderService.java"
    );
    assert_eq!(findings[0].line, Some(17));
    assert_eq!(findings[0].severity, Severity::Blocker);
    assert_eq!(findings[0].source, Source::Compiler);
    assert_eq!(findings[1].severity, Severity::Major);
}

#[test]
fn javac_ignores_lines_that_are_not_diagnostics() {
    let findings = parse::javac("BUILD SUCCESSFUL in 3s\n2 actionable tasks: 2 executed\n");
    assert!(findings.is_empty(), "got: {findings:?}");
}

// ---------------------------------------------------------------------------
// Report discovery
// ---------------------------------------------------------------------------

fn write(path: &std::path::Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("fixture path has a parent")).expect("create dirs");
    fs::write(path, contents).expect("write fixture");
}

/// Gradle writes per-module reports under `<module>/build/reports`. The glob
/// has to reach into a module directory, because a single-module project is the
/// exception in Java, not the rule.
#[test]
fn gradle_reports_are_found_in_a_multi_module_layout() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write(
        &root.join("orders/build/reports/checkstyle/main.xml"),
        CHECKSTYLE_XML,
    );
    write(&root.join("orders/build/reports/pmd/main.xml"), PMD_XML);
    write(
        &root.join("billing/build/reports/spotbugs/main.xml"),
        SPOTBUGS_XML,
    );

    let findings = reports::collect(root, BuildSystem::Gradle, Stage::Analyze);
    assert_eq!(findings.len(), 6, "got: {findings:?}");
    assert!(
        findings.iter().any(|f| f.rule == "SQL_INJECTION_JDBC"),
        "the second module's SpotBugs report was not read: {findings:?}"
    );
}

/// Maven's paths differ from Gradle's for every one of these tools, which is
/// the only thing that varies between the two build systems once a stage has
/// run.
#[test]
fn maven_reports_are_found_at_their_own_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write(&root.join("target/checkstyle-result.xml"), CHECKSTYLE_XML);
    write(&root.join("target/pmd.xml"), PMD_XML);
    write(&root.join("target/spotbugsXml.xml"), SPOTBUGS_XML);

    let findings = reports::collect(root, BuildSystem::Maven, Stage::Analyze);
    assert_eq!(findings.len(), 6, "got: {findings:?}");
}

#[test]
fn gradle_test_results_are_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write(
        &root.join("build/test-results/test/TEST-com.example.OrderServiceTest.xml"),
        JUNIT_XML,
    );
    let findings = reports::collect(root, BuildSystem::Gradle, Stage::Test);
    assert_eq!(findings.len(), 2, "got: {findings:?}");
}

#[test]
fn maven_surefire_results_are_found() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    write(
        &root.join("target/surefire-reports/TEST-com.example.OrderServiceTest.xml"),
        JUNIT_XML,
    );
    let findings = reports::collect(root, BuildSystem::Maven, Stage::Test);
    assert_eq!(findings.len(), 2, "got: {findings:?}");
}

/// An analysis stage that ran and found nothing writes empty reports. That is a
/// different state from "the stage never ran", and both legitimately produce no
/// findings here — the caller separates them using the build's exit code.
#[test]
fn no_reports_yields_no_findings_rather_than_an_error() {
    let temp = tempfile::tempdir().expect("tempdir");
    let findings = reports::collect(temp.path(), BuildSystem::Gradle, Stage::Analyze);
    assert!(findings.is_empty());
}

/// A truncated report is what a killed build leaves behind. Returning what
/// parsed rather than panicking keeps a partial run readable.
#[test]
fn truncated_report_does_not_panic() {
    let truncated = &CHECKSTYLE_XML[..CHECKSTYLE_XML.len() / 2];
    let findings = parse::checkstyle(truncated);
    let _ = findings.len();
}
