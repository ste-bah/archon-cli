//! Where each tool leaves its report, and reading them back.
//!
//! Gradle and Maven put the same tools' output in different places, so the
//! layout is the only thing that differs between the two build systems once a
//! stage has run — every parser downstream of here is shared.

use std::path::Path;

use super::finding::Finding;
use super::parse;
use super::project::{BuildSystem, Stage};

/// A report location: the analyser's name, a glob relative to the project root,
/// and the parser that understands what it finds there.
struct ReportKind {
    /// The analyser, for reporting which ones left nothing behind.
    name: &'static str,
    /// Relative glob. `**/` because both build systems write per-module
    /// reports, and a multi-module project is the normal case in Java.
    pattern: &'static str,
    parse: fn(&str) -> Vec<Finding>,
}

const GRADLE_ANALYSIS: &[ReportKind] = &[
    ReportKind {
        name: "checkstyle",
        pattern: "**/build/reports/checkstyle/*.xml",
        parse: parse::checkstyle,
    },
    ReportKind {
        name: "pmd",
        pattern: "**/build/reports/pmd/*.xml",
        parse: parse::pmd,
    },
    ReportKind {
        name: "spotbugs",
        pattern: "**/build/reports/spotbugs/*.xml",
        parse: parse::spotbugs,
    },
];

const MAVEN_ANALYSIS: &[ReportKind] = &[
    ReportKind {
        name: "checkstyle",
        pattern: "**/target/checkstyle-result.xml",
        parse: parse::checkstyle,
    },
    ReportKind {
        name: "pmd",
        pattern: "**/target/pmd.xml",
        parse: parse::pmd,
    },
    ReportKind {
        name: "spotbugs",
        pattern: "**/target/spotbugsXml.xml",
        parse: parse::spotbugs,
    },
];

const GRADLE_TESTS: &[ReportKind] = &[ReportKind {
    name: "tests",
    pattern: "**/build/test-results/**/TEST-*.xml",
    parse: parse::junit,
}];

const MAVEN_TESTS: &[ReportKind] = &[ReportKind {
    name: "tests",
    pattern: "**/target/surefire-reports/TEST-*.xml",
    parse: parse::junit,
}];

fn kinds_for(build_system: BuildSystem, stage: Stage) -> &'static [ReportKind] {
    match (build_system, stage) {
        // Compile produces no report file — see `collect`.
        (_, Stage::Compile) => &[],
        (BuildSystem::Gradle, Stage::Analyze) => GRADLE_ANALYSIS,
        (BuildSystem::Maven, Stage::Analyze) => MAVEN_ANALYSIS,
        (BuildSystem::Gradle, Stage::Test) => GRADLE_TESTS,
        (BuildSystem::Maven, Stage::Test) => MAVEN_TESTS,
    }
}

/// What a stage's reports contained, and which never appeared.
pub struct StageReports {
    pub findings: Vec<Finding>,
    /// Analysers that left no report file at all.
    ///
    /// This is the distinction that matters most here. An analyser that ran and
    /// found nothing writes an empty report; one that crashed writes none, and
    /// in the findings the two are identical — both contribute zero. SpotBugs
    /// aborting on an unsupported class file version is exactly this shape: the
    /// build still exits zero, the other analysers still report, and a dead
    /// security scanner is invisible.
    ///
    /// An absent report is not proof of breakage — a project that does not
    /// configure PMD will never write a PMD report — so this is surfaced as
    /// information rather than treated as a failure.
    pub missing: Vec<&'static str>,
}

/// Read every report a stage should have written.
pub fn collect_stage(root: &Path, build_system: BuildSystem, stage: Stage) -> StageReports {
    let mut findings = Vec::new();
    let mut missing = Vec::new();

    for kind in kinds_for(build_system, stage) {
        let paths = matching_files(root, kind.pattern);
        if paths.is_empty() {
            missing.push(kind.name);
            continue;
        }
        for path in paths {
            match std::fs::read_to_string(&path) {
                Ok(xml) => findings.extend((kind.parse)(&xml)),
                Err(e) => tracing::warn!("could not read report {}: {e}", path.display()),
            }
        }
    }

    StageReports { findings, missing }
}

/// Findings only, for callers that do not care which analysers were silent.
pub fn collect(root: &Path, build_system: BuildSystem, stage: Stage) -> Vec<Finding> {
    collect_stage(root, build_system, stage).findings
}

/// Files under `root` matching a relative glob.
///
/// `glob` wants one string, and a Windows root contains backslashes that it
/// would read as escapes, so the separators are normalised first.
fn matching_files(root: &Path, pattern: &str) -> Vec<std::path::PathBuf> {
    let root = root.to_string_lossy().replace('\\', "/");
    let full = format!("{}/{pattern}", root.trim_end_matches('/'));
    match glob::glob(&full) {
        Ok(paths) => paths.flatten().collect(),
        Err(e) => {
            tracing::warn!("bad report glob {full}: {e}");
            Vec::new()
        }
    }
}
