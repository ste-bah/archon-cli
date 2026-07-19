use super::{CapturedPatch, PatchError};
use crate::write_coordinator::WriteCoordinatorConfig;
use crate::write_coordinator::write_plan::WritePlan;

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
        validate_complexity(file, &text, cfg.max_function_complexity)?;
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
    if let Some(baseline_lines) = baseline_line_count(baseline)
        && lines <= baseline_lines
    {
        return Ok(());
    }
    Err(PatchError::FileTooManyLines {
        path: path.to_string(),
        lines,
        max,
    })
}

fn baseline_line_count(text: Option<&str>) -> Option<u32> {
    text.map(|value| value.lines().count() as u32)
}

fn validate_complexity(path: &str, text: &str, max: u32) -> Result<(), PatchError> {
    if max == 0 {
        return Ok(());
    }
    for function in function_scores(text) {
        if function.score > max {
            return Err(PatchError::FunctionTooComplex {
                path: path.to_string(),
                function: function.name,
                complexity: function.score,
                max,
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct FunctionScore {
    name: String,
    score: u32,
}

fn function_scores(text: &str) -> Vec<FunctionScore> {
    let mut scores = brace_language_scores(text);
    scores.extend(python_scores(text));
    scores
}

fn brace_language_scores(text: &str) -> Vec<FunctionScore> {
    let mut out = Vec::new();
    let mut active: Option<(String, i32, u32)> = None;
    for raw in text.lines() {
        let line = strip_comment(raw);
        if active.is_none()
            && let Some(name) = brace_function_name(line)
            && line.contains('{')
        {
            active = Some((name, 0, 1));
        }
        let Some((name, depth, score)) = active.as_mut() else {
            continue;
        };
        *score += branch_score(line);
        *depth += brace_delta(line);
        if *depth <= 0 {
            out.push(FunctionScore {
                name: std::mem::take(name),
                score: *score,
            });
            active = None;
        }
    }
    out
}

fn python_scores(text: &str) -> Vec<FunctionScore> {
    let mut out = Vec::new();
    let mut active: Option<(String, usize, u32)> = None;
    for raw in text.lines() {
        let line = strip_comment(raw);
        if line.trim().is_empty() {
            continue;
        }
        let indent = raw.len().saturating_sub(raw.trim_start().len());
        if let Some((name, base, score)) = active.as_mut() {
            if indent <= *base && !line.trim_start().starts_with('@') {
                out.push(FunctionScore {
                    name: std::mem::take(name),
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
            active = Some((name.to_string(), indent, 1));
        }
    }
    if let Some((name, _, score)) = active {
        out.push(FunctionScore { name, score });
    }
    out
}

fn brace_function_name(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if let Some(name) = trimmed.strip_prefix("fn ").and_then(name_before_paren) {
        return Some(name.to_string());
    }
    if let Some(after) = trimmed
        .split_once(" fn ")
        .and_then(|(_, after)| name_before_paren(after))
    {
        return Some(after.to_string());
    }
    let before = trimmed.split_once('(')?.0.trim();
    let name = before.split_whitespace().last()?;
    valid_name(name).then(|| name.to_string())
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

fn branch_score(line: &str) -> u32 {
    let lowered = line.to_ascii_lowercase();
    let logical = lowered.matches("&&").count() + lowered.matches("||").count();
    tokenized(&lowered)
        .filter(|token| {
            matches!(
                *token,
                "if" | "for" | "while" | "match" | "case" | "catch" | "elif" | "except"
            )
        })
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

fn checked_source(path: &str) -> bool {
    let Some((_, ext)) = path.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext,
        "c" | "cc"
            | "cpp"
            | "cs"
            | "go"
            | "h"
            | "hpp"
            | "java"
            | "js"
            | "jsx"
            | "kt"
            | "kts"
            | "mjs"
            | "py"
            | "pyi"
            | "rs"
            | "sh"
            | "swift"
            | "ts"
            | "tsx"
            | "vue"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_function_score_counts_branches() {
        let code = "fn f() { if a { for b in c { while d {} } } }\n";
        let scores = function_scores(code);
        assert_eq!(scores[0].name, "f");
        assert_eq!(scores[0].score, 4);
    }

    #[test]
    fn python_function_score_counts_branches() {
        let code = "def f(x):\n    if x:\n        for y in x:\n            pass\n";
        let scores = function_scores(code);
        assert_eq!(scores[0].name, "f");
        assert_eq!(scores[0].score, 3);
    }
}
