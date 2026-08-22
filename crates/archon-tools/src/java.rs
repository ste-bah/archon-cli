//! Java build-and-analysis toolchain (#176).
//!
//! Drives a Gradle or Maven project through compile, static analysis, security
//! analysis and tests, and reads the tools' **report files** rather than their
//! console output. The reports carry the rule identity and CWE mapping that
//! make a finding actionable; the console carries neither, and its format
//! differs between the two build systems.
//!
//! Stage order is deliberate. Nothing else can run on code that does not
//! compile, and SpotBugs works on bytecode rather than source, so a failed
//! compile short-circuits the run instead of producing a pile of misleading
//! empty reports.
//!
//! The analysis tools themselves are not installed by archon: Checkstyle, PMD,
//! SpotBugs, FindSecBugs, Error Prone and PIT are declared as plugins in the
//! project's own build and fetched by it. That is why the Java system
//! dependency set is exactly JDK, Gradle and Maven.

pub mod diagnose;
pub mod finding;
pub mod invoke;
pub mod parse;
pub mod project;
pub mod reports;

use std::time::Duration;

use serde_json::json;

use crate::tool::{
    PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult, WorkingTreeEffect,
};
use finding::{Finding, select_issues, severity_counts};
use project::{JavaProject, Stage};

/// How many findings are put in front of the model in one pass.
///
/// Presenting a whole report at once measurably degrades the result: the fix
/// quality falls as the list grows. A small, severity-ordered batch is what the
/// loop is built around.
const DEFAULT_ISSUE_LIMIT: usize = 5;

/// Per-stage wall clock. A cold Gradle run resolves plugins over the network
/// and can legitimately take minutes; beyond this it is stuck, not slow.
const DEFAULT_TIMEOUT_SECS: u64 = 900;

pub struct JavaToolchain;

#[async_trait::async_trait]
impl Tool for JavaToolchain {
    fn name(&self) -> &str {
        "JavaToolchain"
    }

    fn capability(&self) -> ToolCapability {
        ToolCapability::HOST_HANDLE
    }

    fn description(&self) -> &str {
        "Drive a Java project's Gradle or Maven build and read its analysis reports. \
         Operations: detect (build system and launcher), compile, analyze \
         (Checkstyle, PMD, SpotBugs/FindSecBugs), test, report (re-read existing \
         reports without running a build). Findings are returned severity-ordered \
         and anchored at their line, with rule keys and CWE identifiers."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["detect", "compile", "analyze", "test", "report"],
                    "description": "detect (identify build system), compile, analyze (static + security), test, report (parse existing reports, run nothing)"
                },
                "path": {
                    "type": "string",
                    "description": "Project root. Defaults to the working directory."
                },
                "limit": {
                    "type": "integer",
                    "description": "How many findings to return, most severe first (default: 5). Large batches measurably reduce fix quality.",
                    "default": DEFAULT_ISSUE_LIMIT
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Per-stage timeout in seconds (default: 900).",
                    "default": DEFAULT_TIMEOUT_SECS
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let Some(operation) = input.get("operation").and_then(|v| v.as_str()) else {
            return ToolResult::error("operation is required");
        };

        let root = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let Some(project) = project::detect(&root) else {
            return ToolResult::error(format!(
                "no Gradle or Maven project at {}: found no settings.gradle[.kts], \
                 build.gradle[.kts] or pom.xml",
                root.display()
            ));
        };

        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_ISSUE_LIMIT as u64) as usize;
        let timeout = Duration::from_secs(
            input
                .get("timeout_secs")
                .and_then(|v| v.as_u64())
                .unwrap_or(DEFAULT_TIMEOUT_SECS),
        );

        match operation {
            "detect" => ToolResult::success(describe(&project)),
            "report" => {
                let mut findings =
                    reports::collect(&project.root, project.build_system, Stage::Analyze);
                findings.extend(reports::collect(
                    &project.root,
                    project.build_system,
                    Stage::Test,
                ));
                ToolResult::success(render_findings(&project, "report", &findings, limit))
            }
            "compile" => run(&project, Stage::Compile, limit, timeout, ctx).await,
            "analyze" => run(&project, Stage::Analyze, limit, timeout, ctx).await,
            "test" => run(&project, Stage::Test, limit, timeout, ctx).await,
            other => ToolResult::error(format!(
                "Unknown operation '{other}'. Valid: detect, compile, analyze, test, report"
            )),
        }
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::Arbitrary
    }

    /// Running a project's own build executes whatever that build declares, so
    /// this is never `Safe` — the same reasoning that makes Bash risky.
    fn permission_level(&self, input: &serde_json::Value) -> PermissionLevel {
        match input.get("operation").and_then(|v| v.as_str()) {
            // Neither starts a process: one stats a few paths, the other reads
            // files that a previous run already wrote.
            Some("detect") | Some("report") => PermissionLevel::Safe,
            _ => PermissionLevel::Risky,
        }
    }
}

fn describe(project: &JavaProject) -> String {
    format!(
        "build system: {}\nroot: {}\nlauncher: {}\n{}",
        project.build_system.as_str(),
        project.root.display(),
        project.launcher.display(),
        match &project.launcher {
            project::Launcher::Wrapper(_) =>
                "using the project's wrapper, which pins the build-tool version the project expects",
            project::Launcher::OnPath(_) =>
                "no wrapper in this project; using the build tool on PATH, which may be a different version than the project was written against",
        }
    )
}

async fn run(
    project: &JavaProject,
    stage: Stage,
    limit: usize,
    timeout: Duration,
    ctx: &ToolContext,
) -> ToolResult {
    let outcome = invoke::run_stage(project, stage, timeout, ctx.cancel_parent.clone()).await;

    if let Some(reason) = &outcome.aborted {
        return ToolResult::error(format!(
            "{} {} did not complete: {reason}",
            project.build_system.as_str(),
            stage.as_str()
        ));
    }

    // Compile is the one stage with no report file to read; its diagnostics
    // only exist in what javac wrote.
    let (findings, missing) = match stage {
        Stage::Compile => (parse::javac(&outcome.output), Vec::new()),
        _ => {
            let collected = reports::collect_stage(&project.root, project.build_system, stage);
            (collected.findings, collected.missing)
        }
    };

    let mut body = render_findings(project, stage.as_str(), &findings, limit);

    // Said plainly, because an analyser that produced nothing contributes
    // exactly as many findings as one that ran clean.
    if !missing.is_empty() {
        body.push_str(&format!(
            "\nNo report from: {}. That analyser is either not configured in \
             this build or failed to run — which is NOT the same as it finding \
             nothing.\n",
            missing.join(", ")
        ));

        // When the build output says why, this stops being a caveat and becomes
        // a definite, fixable fault — so it is returned as an error rather than
        // a successful stage with a note attached. An incomplete security scan
        // that reports success is the failure this whole path exists to avoid.
        if let Some(explanation) = diagnose::explain_missing_reports(&outcome.output) {
            body.push('\n');
            body.push_str(&explanation);
            return ToolResult::error(body);
        }

        body.push_str("Check the build output before treating this stage as clean.\n");
    }

    // A non-zero exit with nothing parsed means the failure is not one the
    // reports describe — a missing plugin, an unresolvable dependency, a broken
    // wrapper. Handing back the console output is the only way to see it.
    if !outcome.succeeded() && findings.is_empty() {
        body.push_str(&format!(
            "\n\nThe build exited {} but no findings were parsed, so the failure is \
             not one the analysis reports describe. Build output follows:\n\n{}",
            outcome.exit_code, outcome.output
        ));
        return ToolResult::error(body);
    }

    ToolResult::success(body)
}

fn render_findings(
    project: &JavaProject,
    stage: &str,
    findings: &[Finding],
    limit: usize,
) -> String {
    if findings.is_empty() {
        return format!(
            "{} {stage}: clean — no findings in the {} reports.",
            project.build_system.as_str(),
            project.build_system.as_str()
        );
    }

    let counts = severity_counts(findings)
        .into_iter()
        .map(|(severity, count)| format!("{count} {severity}"))
        .collect::<Vec<_>>()
        .join(", ");

    let selected = select_issues(findings, limit);
    let mut out = format!(
        "{} {stage}: {} finding(s) — {counts}.\nShowing the {} most severe:\n\n",
        project.build_system.as_str(),
        findings.len(),
        selected.len()
    );
    for finding in &selected {
        out.push_str("  ");
        out.push_str(&finding.render());
        out.push('\n');
    }
    if findings.len() > selected.len() {
        out.push_str(&format!(
            "\n{} more not shown. Fix these first and re-run: a fix can introduce \
             a new violation, so the remaining list is only accurate after the \
             next pass.\n",
            findings.len() - selected.len()
        ));
    }
    out
}
