//! One normalised issue, whatever tool produced it, and how a batch of them is
//! presented back to the model.
//!
//! Checkstyle, PMD, SpotBugs and the compiler all describe severity differently
//! — a Checkstyle `severity` string, a PMD 1–5 priority, a SpotBugs 1–20 rank —
//! so each parser maps onto the scale here and nothing downstream has to know
//! which tool a finding came from.

use std::fmt;

/// Where a finding came from. Kept on the finding because the remedy differs:
/// a compiler error blocks everything, a style violation does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Compiler,
    Checkstyle,
    Pmd,
    SpotBugs,
    Tests,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Compiler => "compiler",
            Source::Checkstyle => "checkstyle",
            Source::Pmd => "pmd",
            Source::SpotBugs => "spotbugs",
            Source::Tests => "tests",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Severity, most serious first. Ordering is derived and load-bearing: issue
/// selection sorts on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Blocker,
    Critical,
    Major,
    Minor,
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Blocker => "blocker",
            Severity::Critical => "critical",
            Severity::Major => "major",
            Severity::Minor => "minor",
            Severity::Info => "info",
        }
    }

    /// PMD priority: 1 is most urgent, 5 least.
    pub fn from_pmd_priority(priority: u32) -> Self {
        match priority {
            1 => Severity::Blocker,
            2 => Severity::Critical,
            3 => Severity::Major,
            4 => Severity::Minor,
            _ => Severity::Info,
        }
    }

    /// SpotBugs rank: 1–20, where 1–4 is "scariest" and 20 "of concern".
    pub fn from_spotbugs_rank(rank: u32) -> Self {
        match rank {
            0..=4 => Severity::Blocker,
            5..=9 => Severity::Critical,
            10..=14 => Severity::Major,
            _ => Severity::Minor,
        }
    }

    /// Checkstyle severity strings, which are configurable per module but in
    /// practice are these three.
    pub fn from_checkstyle(severity: &str) -> Self {
        match severity {
            "error" => Severity::Critical,
            "warning" => Severity::Major,
            _ => Severity::Info,
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single issue, located and attributed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Path as the tool reported it — absolute for most Java tooling.
    pub file: String,
    /// Absent for findings that describe a whole file or a whole test class.
    pub line: Option<u32>,
    pub severity: Severity,
    /// Rule identity: a Checkstyle module, a PMD rule name, a SpotBugs bug
    /// pattern, or a compiler diagnostic key. This is what a reader looks up.
    pub rule: String,
    /// CWE identifier where the tool supplies one. FindSecBugs populates this
    /// for its security patterns; the style checkers never do.
    pub cwe: Option<String>,
    pub message: String,
    pub source: Source,
}

impl Finding {
    /// Render as one line, anchored at the location the tool reported.
    pub fn render(&self) -> String {
        let location = match self.line {
            Some(line) => format!("{}:{line}", self.file),
            None => self.file.clone(),
        };
        let cwe = match &self.cwe {
            Some(cwe) => format!(" [{cwe}]"),
            None => String::new(),
        };
        format!(
            "{location}  {} [{}/{}]{cwe} {}",
            self.severity, self.source, self.rule, self.message
        )
    }
}

/// Choose which findings to put in front of the model this pass.
///
/// Presenting an entire report at once measurably degrades the result, so this
/// takes the most severe `limit` findings rather than all of them. Ordering is
/// severity first, then file and line, so a batch stays in a stable, readable
/// order rather than whatever order the reports were parsed in.
pub fn select_issues(findings: &[Finding], limit: usize) -> Vec<Finding> {
    let mut ordered: Vec<Finding> = findings.to_vec();
    ordered.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.rule.cmp(&b.rule))
    });
    ordered.truncate(limit);
    ordered
}

/// Count of findings per severity, most serious first, for a one-line summary.
pub fn severity_counts(findings: &[Finding]) -> Vec<(Severity, usize)> {
    let ladder = [
        Severity::Blocker,
        Severity::Critical,
        Severity::Major,
        Severity::Minor,
        Severity::Info,
    ];
    ladder
        .into_iter()
        .filter_map(|severity| {
            let count = findings.iter().filter(|f| f.severity == severity).count();
            (count > 0).then_some((severity, count))
        })
        .collect()
}
