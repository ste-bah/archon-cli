use super::{CapturedPatch, PatchError};
use crate::write_coordinator::WriteCoordinatorConfig;
use crate::write_coordinator::write_plan::WritePlan;

mod brace_scan;
#[cfg(test)]
mod brace_scan_tests;
mod complexity_ratchet;
#[cfg(test)]
mod complexity_ratchet_tests;

use complexity_ratchet::validate_complexity;

pub(super) fn validate(
    captured: &CapturedPatch,
    plan: &WritePlan,
    cfg: &WriteCoordinatorConfig,
) -> Result<(), PatchError> {
    for file in &captured.changed_files {
        if captured.deleted_files.contains(file) || !checked_source(file) {
            continue;
        }
        let path = plan.isolated_root.join(file);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let baseline = std::fs::read_to_string(plan.canonical_root.join(file)).ok();
        validate_line_count(file, baseline.as_deref(), &text, cfg.max_source_file_lines)?;
        validate_complexity(
            file,
            baseline.as_deref(),
            &text,
            cfg.max_function_complexity,
        )?;
    }
    Ok(())
}

fn validate_line_count(
    path: &str,
    baseline: Option<&str>,
    text: &str,
    max: u32,
) -> Result<(), PatchError> {
    if max == 0 {
        return Ok(());
    }
    let lines = text.lines().count() as u32;
    if lines <= max {
        return Ok(());
    }
    let baseline_lines = baseline_line_count(baseline);
    if let Some(baseline_lines) = baseline_lines
        && lines <= baseline_lines
    {
        return Ok(());
    }
    Err(PatchError::FileTooManyLines {
        path: path.to_string(),
        lines,
        // The CURRENT size, so the message cannot be read as a statement about
        // the file. `lines` is the hypothetical post-patch count, and reporting
        // it alone made a 483-line file look like a 690-line one — a
        // misdiagnosis that cost two agent rounds and misled two humans with
        // full repo access on three separate occasions in one day.
        baseline: baseline_lines.unwrap_or(0),
        max,
        module_dir: module_directory_for(path),
    })
}

/// The module directory a declared target owns: `a/b.rs` owns `a/b/`.
///
/// Named in the rejection because "split this file" is a refactor an agent
/// cannot land inside one remediation round, while "put the new code in
/// `a/b/`" is a single-round action. Observed live: agents trimmed an
/// oversized addition twice rather than relocating it, because the message
/// asked for a split and never said where new code could go.
fn module_directory_for(path: &str) -> String {
    match path.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}/"),
        None => format!("{path}/"),
    }
}

fn baseline_line_count(text: Option<&str>) -> Option<u32> {
    text.map(|value| value.lines().count() as u32)
}

#[derive(Debug, Clone)]
struct FunctionScore {
    name: String,
    /// 1-based line of the function's header (its name), so a rejection
    /// points at the function even when two share a name.
    line: usize,
    score: u32,
}

/// Every function's score in `text`. `path` selects the header forms: a
/// `.rs` file declares functions only with `fn`.
fn function_scores(path: &str, text: &str) -> Vec<FunctionScore> {
    let rust = path.ends_with(".rs");
    let mut scores = brace_scan::brace_language_scores(text.lines().map(strip_comment), rust);
    scores.extend(python_scores(text));
    scores
}

fn python_scores(text: &str) -> Vec<FunctionScore> {
    let mut out = Vec::new();
    let mut active: Option<(String, usize, usize, u32)> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = strip_comment(raw);
        if line.trim().is_empty() {
            continue;
        }
        let indent = raw.len().saturating_sub(raw.trim_start().len());
        if let Some((name, start, base, score)) = active.as_mut() {
            if indent <= *base && !line.trim_start().starts_with('@') {
                out.push(FunctionScore {
                    name: std::mem::take(name),
                    line: *start,
                    score: *score,
                });
                active = None;
            } else {
                *score += branch_score(line);
            }
        }
        if active.is_none()
            && let Some(name) = line
                .trim_start()
                .strip_prefix("def ")
                .and_then(name_before_paren)
        {
            active = Some((name.to_string(), index + 1, indent, 1));
        }
    }
    if let Some((name, line, _, score)) = active {
        out.push(FunctionScore { name, line, score });
    }
    out
}

fn name_before_paren(text: &str) -> Option<&str> {
    text.split_once('(')
        .map(|(name, _)| name.trim())
        .filter(|name| valid_name(name))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !matches!(
            name,
            "if" | "for" | "while" | "switch" | "match" | "catch" | "return"
        )
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '<' | '>'))
}

/// The tokens that add one to a function's score, each occurrence, after the
/// line is lower-cased and its `//` / `#` comment tail is stripped. Named
/// here and quoted to the coder (`v2::write::landing_policy`) so the prompt
/// describes the metric this file computes, not a textbook one.
pub(crate) const BRANCH_TOKENS: &[&str] = &[
    "if", "for", "while", "match", "case", "catch", "elif", "except",
];

/// The operators that add one each, alongside [`BRANCH_TOKENS`].
pub(crate) const LOGICAL_OPERATORS: &[&str] = &["&&", "||"];

fn branch_score(line: &str) -> u32 {
    let lowered = line.to_ascii_lowercase();
    let logical: usize = LOGICAL_OPERATORS
        .iter()
        .map(|operator| lowered.matches(operator).count())
        .sum();
    tokenized(&lowered)
        .filter(|token| BRANCH_TOKENS.contains(token))
        .count() as u32
        + logical as u32
}

fn tokenized(line: &str) -> impl Iterator<Item = &str> {
    line.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .filter(|token| !token.is_empty())
}

fn brace_delta(line: &str) -> i32 {
    let opens = line.chars().filter(|ch| *ch == '{').count() as i32;
    let closes = line.chars().filter(|ch| *ch == '}').count() as i32;
    opens - closes
}

fn strip_comment(line: &str) -> &str {
    line.split("//")
        .next()
        .unwrap_or(line)
        .split('#')
        .next()
        .unwrap_or(line)
}

/// The file extensions the line and complexity caps apply to. Any other
/// changed file is subject only to the byte caps.
pub(crate) const CHECKED_SOURCE_EXTENSIONS: &[&str] = &[
    "c", "cc", "cpp", "cs", "go", "h", "hpp", "java", "js", "jsx", "kt", "kts", "mjs", "py", "pyi",
    "rs", "sh", "swift", "ts", "tsx", "vue",
];

fn checked_source(path: &str) -> bool {
    let Some((_, ext)) = path.rsplit_once('.') else {
        return false;
    };
    CHECKED_SOURCE_EXTENSIONS.contains(&ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_function_score_counts_branches() {
        let code = "fn f() { if a { for b in c { while d {} } } }\n";
        let scores = function_scores("a.rs", code);
        assert_eq!(scores[0].name, "f");
        assert_eq!(scores[0].score, 4);
    }

    #[test]
    fn python_function_score_counts_branches() {
        let code = "def f(x):\n    if x:\n        for y in x:\n            pass\n";
        let scores = function_scores("a.py", code);
        assert_eq!(scores[0].name, "f");
        assert_eq!(scores[0].line, 1);
        assert_eq!(scores[0].score, 3);
    }
}
