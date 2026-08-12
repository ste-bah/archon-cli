//! Where each tool leaves its report, and reading them back.
//!
//! Gradle and Maven put the same tools' output in different places, so the
//! layout is the only thing that differs between the two build systems once a
//! stage has run — every parser downstream of here is shared.

use std::path::Path;

use super::finding::Finding;
use super::parse;
use super::project::{BuildSystem, Stage};

/// A report location: a glob relative to the project root, and the parser that
/// understands what it finds there.
struct ReportKind {
    /// Relative glob. `**/` because both build systems write per-module
    /// reports, and a multi-module project is the normal case in Java.
    pattern: &'static str,
    parse: fn(&str) -> Vec<Finding>,
}

const GRADLE_ANALYSIS: &[ReportKind] = &[
    ReportKind {
        pattern: "**/build/reports/checkstyle/*.xml",
        parse: parse::checkstyle,
    },
    ReportKind {
        pattern: "**/build/reports/pmd/*.xml",
        parse: parse::pmd,
    },
    ReportKind {
        pattern: "**/build/reports/spotbugs/*.xml",
        parse: parse::spotbugs,
    },
];

const MAVEN_ANALYSIS: &[ReportKind] = &[
    ReportKind {
        pattern: "**/target/checkstyle-result.xml",
        parse: parse::checkstyle,
    },
    ReportKind {
        pattern: "**/target/pmd.xml",
        parse: parse::pmd,
    },
    ReportKind {
        pattern: "**/target/spotbugsXml.xml",
        parse: parse::spotbugs,
    },
];

const GRADLE_TESTS: &[ReportKind] = &[ReportKind {
    pattern: "**/build/test-results/**/TEST-*.xml",
    parse: parse::junit,
}];

const MAVEN_TESTS: &[ReportKind] = &[ReportKind {
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

/// Read every report a stage should have written and return the findings.
///
/// A stage that ran cleanly writes reports containing nothing, so an empty
/// result here means "the tools found nothing", not "the tools did not run" —
/// the two are distinguished by the caller, which knows whether the stage
/// exited non-zero.
pub fn collect(root: &Path, build_system: BuildSystem, stage: Stage) -> Vec<Finding> {
    let mut findings = Vec::new();
    for kind in kinds_for(build_system, stage) {
        for path in matching_files(root, kind.pattern) {
            match std::fs::read_to_string(&path) {
                Ok(xml) => findings.extend((kind.parse)(&xml)),
                Err(e) => tracing::warn!("could not read report {}: {e}", path.display()),
            }
        }
    }
    findings
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
