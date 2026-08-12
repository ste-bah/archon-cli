//! Report-file parsers for the JVM analysis tools.
//!
//! Every one of these tools writes a machine-readable report and also prints a
//! human summary to the console. The reports are what get read here: console
//! output is formatted for a terminal, differs between Gradle and Maven, and
//! carries neither the rule identity nor the CWE mapping that makes a finding
//! actionable.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use super::finding::{Finding, Severity, Source};

/// Decode raw XML bytes and resolve entity references.
///
/// `Attribute::unescape_value` is not used: it is compiled out whenever any
/// crate in the graph turns on quick-xml's `encoding` feature, and Cargo
/// unifies features across the workspace, so relying on it makes this file's
/// compilation depend on an unrelated dependency's choices.
fn unescaped(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    match quick_xml::escape::unescape(&text) {
        Ok(value) => value.into_owned(),
        // An unresolvable entity is not a reason to drop the finding; the
        // escaped form still names the file, rule and line.
        Err(_) => text.into_owned(),
    }
}

/// Value of an attribute, unescaped.
fn attr(element: &BytesStart, name: &[u8]) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .map(|a| unescaped(&a.value))
}

fn attr_u32(element: &BytesStart, name: &[u8]) -> Option<u32> {
    attr(element, name)?.parse().ok()
}

fn reader_for(xml: &str) -> Reader<&[u8]> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    reader
}

/// Checkstyle's XML report.
///
/// ```xml
/// <checkstyle><file name="…/Foo.java">
///   <error line="12" severity="warning" message="…" source="…EmptyStatementCheck"/>
/// </file></checkstyle>
/// ```
pub fn checkstyle(xml: &str) -> Vec<Finding> {
    let mut reader = reader_for(xml);
    let mut findings = Vec::new();
    let mut current_file = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"file" => current_file = attr(&e, b"name").unwrap_or_default(),
                b"error" => findings.push(Finding {
                    file: current_file.clone(),
                    line: attr_u32(&e, b"line"),
                    severity: Severity::from_checkstyle(
                        attr(&e, b"severity").unwrap_or_default().as_str(),
                    ),
                    rule: checkstyle_rule(attr(&e, b"source").as_deref()),
                    cwe: None,
                    message: attr(&e, b"message").unwrap_or_default(),
                    source: Source::Checkstyle,
                }),
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => {
                tracing::warn!("malformed Checkstyle report: {e}");
                break;
            }
            _ => {}
        }
    }
    findings
}

/// `com.puppycrawl.tools.checkstyle.checks.coding.EmptyStatementCheck`
/// carries no information a reader wants except `EmptyStatement`.
fn checkstyle_rule(source: Option<&str>) -> String {
    let Some(source) = source else {
        return "checkstyle".to_string();
    };
    let leaf = source.rsplit('.').next().unwrap_or(source);
    leaf.strip_suffix("Check").unwrap_or(leaf).to_string()
}

/// PMD's XML report. Unlike the others the message is element text rather than
/// an attribute, so it is accumulated between the open and close tags.
///
/// ```xml
/// <pmd><file name="…/Foo.java">
///   <violation beginline="10" rule="EmptyCatchBlock" priority="3">message</violation>
/// </file></pmd>
/// ```
pub fn pmd(xml: &str) -> Vec<Finding> {
    let mut reader = reader_for(xml);
    let mut findings = Vec::new();
    let mut current_file = String::new();
    let mut open: Option<Finding> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"file" => current_file = attr(&e, b"name").unwrap_or_default(),
                b"violation" => {
                    open = Some(Finding {
                        file: current_file.clone(),
                        line: attr_u32(&e, b"beginline"),
                        severity: Severity::from_pmd_priority(
                            attr_u32(&e, b"priority").unwrap_or(3),
                        ),
                        rule: attr(&e, b"rule").unwrap_or_else(|| "pmd".to_string()),
                        cwe: None,
                        message: String::new(),
                        source: Source::Pmd,
                    });
                }
                _ => {}
            },
            Ok(Event::Text(text)) => {
                if let Some(finding) = open.as_mut() {
                    finding.message.push_str(unescaped(text.as_ref()).trim());
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"violation"
                    && let Some(finding) = open.take()
                {
                    findings.push(finding);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                tracing::warn!("malformed PMD report: {e}");
                break;
            }
            _ => {}
        }
    }
    findings
}

/// SpotBugs' XML report, which is also what FindSecBugs writes into.
///
/// The `cweid` attribute is the reason this pipeline exists in the shape it
/// does: it is what turns "SpotBugs is unhappy" into a named weakness that maps
/// onto the OWASP and SANS lists.
///
/// A `BugInstance` carries several `SourceLine` elements (the class, the
/// method, the specific expression); the first is the one that locates the bug.
pub fn spotbugs(xml: &str) -> Vec<Finding> {
    let mut reader = reader_for(xml);
    let mut findings = Vec::new();
    let mut open: Option<Finding> = None;
    let mut located = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"BugInstance" => {
                    located = false;
                    // Rank is the calibrated 1-20 scale; priority (1-3) is the
                    // older confidence value and is not a severity.
                    let mut severity =
                        Severity::from_spotbugs_rank(attr_u32(&e, b"rank").unwrap_or(20));
                    // SpotBugs ranks by how likely the report is to be a real
                    // defect, not by what it costs when it is. A live SQL
                    // injection comes back at rank 12, which on the rank scale
                    // alone is "major" — the same band as an unused variable.
                    // A finding in the SECURITY category is floored at critical
                    // so it cannot be sorted below a style violation.
                    if attr(&e, b"category").as_deref() == Some("SECURITY")
                        && severity > Severity::Critical
                    {
                        severity = Severity::Critical;
                    }
                    open = Some(Finding {
                        file: String::new(),
                        line: None,
                        severity,
                        rule: attr(&e, b"type").unwrap_or_else(|| "spotbugs".to_string()),
                        cwe: attr_u32(&e, b"cweid").map(|id| format!("CWE-{id}")),
                        message: String::new(),
                        source: Source::SpotBugs,
                    });
                }
                b"SourceLine" => {
                    if let Some(finding) = open.as_mut()
                        && !located
                    {
                        // sourcepath is package-relative and stable across
                        // machines; sourcefile is the bare file name.
                        if let Some(path) =
                            attr(&e, b"sourcepath").or_else(|| attr(&e, b"sourcefile"))
                        {
                            finding.file = path;
                        }
                        finding.line = attr_u32(&e, b"start");
                        located = true;
                    }
                }
                _ => {}
            },
            Ok(Event::Text(text)) => {
                if let Some(finding) = open.as_mut()
                    && finding.message.is_empty()
                {
                    finding.message = unescaped(text.as_ref()).trim().to_string();
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"BugInstance"
                    && let Some(finding) = open.take()
                {
                    findings.push(finding);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                tracing::warn!("malformed SpotBugs report: {e}");
                break;
            }
            _ => {}
        }
    }
    findings
}

/// Compiler diagnostics, from captured output rather than a report file.
///
/// This is the one stage with no machine-readable artefact to read: `javac`
/// writes diagnostics to stderr and neither Gradle nor Maven persists them.
/// The format is stable and specified — `path:line: error: message` — so it is
/// parsed here rather than handed back as a wall of console text, which would
/// leave compile the only stage whose findings could not be located at a line.
pub fn javac(output: &str) -> Vec<Finding> {
    output
        .lines()
        .filter_map(|line| {
            let (path, rest) = line.split_once(".java:")?;
            let (line_no, rest) = rest.split_once(':')?;
            let line_no: u32 = line_no.trim().parse().ok()?;
            let rest = rest.trim_start();
            let (kind, message) = rest.split_once(':')?;
            let severity = match kind.trim() {
                "error" => Severity::Blocker,
                "warning" => Severity::Major,
                _ => return None,
            };
            Some(Finding {
                file: format!("{path}.java"),
                line: Some(line_no),
                severity,
                rule: format!("javac-{}", kind.trim()),
                cwe: None,
                message: message.trim().to_string(),
                source: Source::Compiler,
            })
        })
        .collect()
}

/// A JUnit/Surefire `TEST-*.xml` result file.
///
/// Only failures and errors become findings. A passing test is not an issue,
/// and emitting one per passing test would drown the batch that matters.
pub fn junit(xml: &str) -> Vec<Finding> {
    let mut reader = reader_for(xml);
    let mut findings = Vec::new();
    let mut current_case = String::new();
    let mut current_class = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"testcase" => {
                    current_case = attr(&e, b"name").unwrap_or_default();
                    current_class = attr(&e, b"classname").unwrap_or_default();
                }
                b"failure" | b"error" => {
                    let kind = if e.name().as_ref() == b"error" {
                        "error"
                    } else {
                        "failure"
                    };
                    findings.push(Finding {
                        file: current_class.clone(),
                        line: None,
                        // A red test blocks everything downstream of it, so
                        // both kinds land at the top of the ladder.
                        severity: Severity::Blocker,
                        rule: format!("test-{kind}"),
                        cwe: None,
                        message: format!(
                            "{current_case}: {}",
                            attr(&e, b"message").unwrap_or_else(|| {
                                attr(&e, b"type").unwrap_or_else(|| "failed".to_string())
                            })
                        ),
                        source: Source::Tests,
                    });
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => {
                tracing::warn!("malformed JUnit report: {e}");
                break;
            }
            _ => {}
        }
    }
    findings
}
