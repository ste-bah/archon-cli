use super::{CapturedPatch, PatchError};
use crate::write_coordinator::WriteCoordinatorConfig;
use crate::write_coordinator::write_plan::WritePlan;

mod brace_headers;
mod brace_scan;
#[cfg(test)]
mod brace_scan_tests;
mod complexity_ratchet;
#[cfg(test)]
mod complexity_ratchet_tests;
#[cfg(test)]
mod scanner_edge_tests;
mod source_text;
#[cfg(test)]
mod sync_and_literal_tests;
mod tree_names;
mod tree_scan;
#[cfg(test)]
mod tree_scan_c_ruby_tests;
#[cfg(test)]
mod tree_scan_holes_tests;
#[cfg(test)]
mod tree_scan_review3_tests;
#[cfg(test)]
mod tree_scan_tests;
mod tree_score;

use complexity_ratchet::validate_complexity;

pub(super) fn validate(
    captured: &CapturedPatch,
    plan: &WritePlan,
    cfg: &WriteCoordinatorConfig,
) -> Result<Vec<super::UnreliableScan>, PatchError> {
    let mut notes = Vec::new();
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
        notes.extend(validate_complexity(
            file,
            baseline.as_deref(),
            &text,
            cfg.max_function_complexity,
        )?);
    }
    Ok(notes)
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
    /// The declaration text up to the body — comments dropped, literal
    /// contents kept, whitespace removed: how the ratchet tells apart
    /// functions that share a name.
    header: String,
    /// False when the reading of this function may be wrong (its syntax
    /// tree holds an error).
    reliable: bool,
}

/// What the complexity gate could read in one file.
#[derive(Debug, Default)]
struct FileScan {
    functions: Vec<FunctionScore>,
    /// The language label reported with any unreliable reading.
    language: String,
    /// Where the reading was unreliable, and why: (1-based line, reason).
    unreliable: Vec<(usize, String)>,
    /// The hand scanner ended inside a function: every span from there on
    /// is unreliable.
    lost_sync: bool,
    /// Some function may be missing from `functions` (lost sync, or a syntax
    /// error outside every function).
    incomplete: bool,
    /// Read from a syntax tree rather than by the hand scanner.
    parsed: bool,
}

/// Every function in `text`, read from a syntax tree where the language has
/// a grammar (`tree_scan`), else by the hand scanner.
fn scan_functions(path: &str, text: &str) -> FileScan {
    if let Some(grammar) = tree_scan::Grammar::for_path(path)
        && let Some(tree) = tree_scan::tree_scan(grammar, text)
    {
        let mut unreliable: Vec<(usize, String)> = tree
            .functions
            .iter()
            .filter(|function| !function.reliable)
            .map(|function| {
                let reason = format!(
                    "the parser could not read part of function '{}' (often a macro)",
                    function.name
                );
                (function.line, reason)
            })
            .collect();
        unreliable.extend(tree.stray_errors.iter().map(|line| {
            let reason = "syntax error outside any function; a function there may be unread";
            (*line, reason.to_string())
        }));
        unreliable.extend(tree.notes);
        return FileScan {
            functions: tree.functions,
            language: tree.grammar.unwrap_or(grammar).label().to_string(),
            incomplete: !tree.stray_errors.is_empty(),
            lost_sync: false,
            parsed: true,
            unreliable,
        };
    }
    hand_scan(path, text)
}

/// The baseline reading the ratchet pairs against. A syntax tree with an
/// error outside every function may be missing a function (valid code the
/// grammar does not know — a newer edition's syntax, a macro — reads this
/// way), so the hand scanner's functions whose names the tree lacks are
/// added to it. The reading stays `incomplete`, so a post-patch function
/// with no counterpart in either is still excused when its name appears in
/// the baseline text.
fn baseline_scan(path: &str, text: &str) -> FileScan {
    let mut tree = scan_functions(path, text);
    if !tree.parsed || !tree.incomplete {
        return tree;
    }
    let hand = hand_scan(path, text);
    let known: std::collections::BTreeSet<String> = tree
        .functions
        .iter()
        .map(|function| function.name.clone())
        .collect();
    tree.functions.extend(
        hand.functions
            .into_iter()
            .filter(|function| !known.contains(&function.name)),
    );
    for (_, reason) in &mut tree.unreliable {
        reason.push_str("; paired against the hand scanner's reading as well");
    }
    tree
}

/// The hand scanner's reading: brace spans plus Python-style indent spans.
fn hand_scan(path: &str, text: &str) -> FileScan {
    let syntax = source_text::syntax_for(path);
    let lines = source_text::code_lines(text, syntax);
    let rust = syntax == source_text::Syntax::Rust;
    let scan = brace_scan::brace_language_scores(&lines, rust);
    let mut functions = scan.functions;
    functions.extend(python_scores(text));
    let unreliable: Vec<(usize, String)> = scan
        .unclosed
        .iter()
        .map(|(name, line)| {
            let reason = format!("scanner lost sync: function '{name}' never closed");
            (*line, reason)
        })
        .collect();
    let lost_sync = !unreliable.is_empty();
    FileScan {
        functions,
        language: path.rsplit_once('.').map_or("", |(_, ext)| ext).to_string(),
        unreliable,
        lost_sync,
        incomplete: lost_sync,
        parsed: false,
    }
}

/// The functions the hand scanner read (tests of that scanner).
#[cfg(test)]
fn function_scores(path: &str, text: &str) -> Vec<FunctionScore> {
    hand_scan(path, text).functions
}

/// `text` with whitespace removed and a trailing comma before `)` dropped,
/// so rewrapping a signature does not change it.
fn normalized_header(text: &str) -> String {
    let compact: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    compact.replace(",)", ")")
}

fn python_scores(text: &str) -> Vec<FunctionScore> {
    let mut out = Vec::new();
    let mut active: Option<(String, usize, usize, u32, String)> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = strip_comment(raw);
        if line.trim().is_empty() {
            continue;
        }
        let indent = raw.len().saturating_sub(raw.trim_start().len());
        if let Some((name, start, base, score, header)) = active.as_mut() {
            if indent <= *base && !line.trim_start().starts_with('@') {
                out.push(FunctionScore {
                    name: std::mem::take(name),
                    line: *start,
                    score: *score,
                    header: std::mem::take(header),
                    reliable: true,
                });
                active = None;
            } else {
                *score += branch_score(line);
            }
        }
        let trimmed = line.trim_start();
        let def = trimmed
            .strip_prefix("async def ")
            .or_else(|| trimmed.strip_prefix("def "));
        if active.is_none()
            && let Some(name) = def.and_then(name_before_paren)
        {
            let header = normalized_header(trimmed);
            active = Some((name.to_string(), index + 1, indent, 1, header));
        }
    }
    if let Some((name, line, _, score, header)) = active {
        out.push(FunctionScore {
            name,
            line,
            score,
            header,
            reliable: true,
        });
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
/// line is lower-cased and its comments (and, in brace languages, its string
/// and character literal contents) are removed. Named
/// here and quoted to the coder (`v2::write::landing_policy`) so the prompt
/// describes the metric this file computes, not a textbook one.
pub(crate) const BRANCH_TOKENS: &[&str] = &[
    "if", "for", "while", "match", "case", "catch", "elif", "except",
];

/// The operators that add one each, alongside [`BRANCH_TOKENS`].
pub(crate) const LOGICAL_OPERATORS: &[&str] = &["&&", "||"];

/// Ruby's further branch keywords and word operators, each adding one like
/// [`BRANCH_TOKENS`]: its spellings of `elif`, `if`, `while`, `case` and
/// `catch`, and `and` / `or` beside `&&` / `||`.
pub(crate) const RUBY_BRANCH_TOKENS: &[&str] =
    &["elsif", "unless", "until", "when", "rescue", "and", "or"];

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
    "c", "cc", "cjs", "cpp", "cs", "cts", "cxx", "go", "h", "hh", "hpp", "hxx", "inl", "ipp",
    "java", "js", "jsx", "kt", "kts", "mjs", "mts", "py", "pyi", "rake", "rb", "rs", "sh", "swift",
    "tpp", "ts", "tsx", "vue",
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
