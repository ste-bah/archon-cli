//! Coding quality calculator — scores agent output across 5 weighted dimensions.
//!
//! Implements REQ-CODE-005: stateless quality analysis producing a composite score
//! from code quality, completeness, structural integrity, documentation, and test coverage.

use regex::Regex;

/// Weighted breakdown of quality across 5 dimensions.
#[derive(Debug, Clone)]
pub struct QualityBreakdown {
    /// Anti-pattern detection, code structure analysis (weight 0.30).
    pub code_quality: f64,
    /// TODO / stub / placeholder / unimplemented detection (weight 0.25).
    pub completeness: f64,
    /// Import resolution, module wiring, dead code detection (weight 0.20).
    pub structural_integrity: f64,
    /// Docstring / comment presence, public API documentation (weight 0.15).
    pub documentation: f64,
    /// Test file references, assertion counts, test names (weight 0.10).
    pub test_coverage: f64,
    /// Weighted sum of the above, rounded to 3 decimal places.
    pub composite: f64,
}

/// Return the minimum quality threshold for a given pipeline phase.
pub fn phase_threshold(phase: u32) -> f64 {
    match phase {
        1 => 0.75,
        2 => 0.80,
        3 => 0.82,
        4 => 0.85,
        5 => 0.88,
        6 => 0.95,
        _ => 0.80,
    }
}

/// Stateless quality calculator that analyses agent output text.
pub struct CodingQualityCalculator {
    re_unwrap: Regex,
    re_unwrap_safe: Regex,
    re_clone_hot: Regex,
    re_long_fn: Regex,
    re_magic_number: Regex,
    re_todo_markers: Regex,
    re_unimplemented: Regex,
    re_stub_markers: Regex,
    re_empty_fn: Regex,
    re_panic_non_test: Regex,
    re_use_stmt: Regex,
    re_allow_dead: Regex,
    re_wire_marker: Regex,
    re_mod_decl: Regex,
    re_doc_comment: Regex,
    re_module_doc: Regex,
    re_doc_attr: Regex,
    re_pub_item: Regex,
    re_test_attr: Regex,
    re_cfg_test: Regex,
    re_assert: Regex,
    re_test_fn_name: Regex,
}

impl CodingQualityCalculator {
    /// Create a new calculator with pre-compiled regex patterns.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            re_unwrap: Regex::new(r"\.unwrap\(\)").expect("valid regex"),
            re_unwrap_safe: Regex::new(r"//\s*(?i:safety|safe):").expect("valid regex"),
            re_clone_hot: Regex::new(
                r"(?:for\s|\.iter\(\)|\.map\(|loop\s*\{|while\s)[\s\S]{0,200}\.clone\(\)",
            )
            .expect("valid regex"),
            re_long_fn: Regex::new(r"(?m)^\s*(?:pub\s+)?(?:async\s+)?fn\s+\w+")
                .expect("valid regex"),
            re_magic_number: Regex::new(r"\b(\d+)\b").expect("valid regex"),
            re_todo_markers: Regex::new(r"(?i)\b(?:TODO|FIXME|HACK|XXX)\b").expect("valid regex"),
            re_unimplemented: Regex::new(r"\b(?:unimplemented|todo)!\(\)").expect("valid regex"),
            re_stub_markers: Regex::new(r"(?i)\b(?:stub|placeholder|not yet implemented)\b")
                .expect("valid regex"),
            re_empty_fn: Regex::new(r"fn\s+\w+[^}]*\{\s*\}").expect("valid regex"),
            re_panic_non_test: Regex::new(r"\bpanic!\(").expect("valid regex"),
            re_use_stmt: Regex::new(r"(?m)^use\s+[\w:]+(?:::\{[^}]+\}|::\w+);")
                .expect("valid regex"),
            re_allow_dead: Regex::new(r"#\[allow\(dead_code\)\]").expect("valid regex"),
            re_wire_marker: Regex::new(r"(?i)//\s*TODO:\s*wire").expect("valid regex"),
            re_mod_decl: Regex::new(r"(?m)^(?:pub\s+)?mod\s+\w+").expect("valid regex"),
            re_doc_comment: Regex::new(r"(?m)^\s*///").expect("valid regex"),
            re_module_doc: Regex::new(r"(?m)^\s*//!").expect("valid regex"),
            re_doc_attr: Regex::new(r#"#\[doc\s*="#).expect("valid regex"),
            re_pub_item: Regex::new(
                r"(?m)^\s*pub\s+(?:fn|struct|enum|trait|type|const|static|mod)\s+\w+",
            )
            .expect("valid regex"),
            re_test_attr: Regex::new(r"#\[test\]").expect("valid regex"),
            re_cfg_test: Regex::new(r"#\[cfg\(test\)\]").expect("valid regex"),
            re_assert: Regex::new(r"\b(?:assert!|assert_eq!|assert_ne!)").expect("valid regex"),
            re_test_fn_name: Regex::new(r"fn\s+test_\w+").expect("valid regex"),
        }
    }

    /// Score an agent's output text across all 5 dimensions.
    pub fn score(&self, output: &str) -> QualityBreakdown {
        if output.trim().is_empty() {
            return QualityBreakdown {
                code_quality: 0.0,
                completeness: 0.0,
                structural_integrity: 0.0,
                documentation: 0.0,
                test_coverage: 0.0,
                composite: 0.0,
            };
        }

        let code_quality = self.score_code_quality(output);
        let completeness = self.score_completeness(output);
        let structural_integrity = self.score_structural_integrity(output);
        let documentation = self.score_documentation(output);
        let test_coverage = self.score_test_coverage(output);

        let raw = code_quality * 0.30
            + completeness * 0.25
            + structural_integrity * 0.20
            + documentation * 0.15
            + test_coverage * 0.10;

        let composite = (raw * 1000.0).round() / 1000.0;

        QualityBreakdown {
            code_quality,
            completeness,
            structural_integrity,
            documentation,
            test_coverage,
            composite,
        }
    }

    /// Check whether output meets the quality threshold for a given phase.
    pub fn meets_threshold(&self, output: &str, phase: u32) -> bool {
        self.score(output).composite >= phase_threshold(phase)
    }

    // ── Code Quality (0.30) ──────────────────────────────────────────

    fn score_code_quality(&self, output: &str) -> f64 {
        let mut score: f64 = 1.0;
        let lines: Vec<&str> = output.lines().collect();

        // Penalty: unwrap() without safety comment
        let mut unwrap_penalty = 0.0_f64;
        for (i, line) in lines.iter().enumerate() {
            let unwrap_count = self.re_unwrap.find_iter(line).count();
            if unwrap_count > 0 && !self.re_unwrap_safe.is_match(line) {
                // Also check the line above for a safety comment
                let prev_safe = if i > 0 {
                    self.re_unwrap_safe.is_match(lines[i - 1])
                } else {
                    false
                };
                if !prev_safe {
                    unwrap_penalty += 0.05 * unwrap_count as f64;
                }
            }
        }
        score -= unwrap_penalty.min(0.5);

        // Penalty: clone() in hot paths
        let clone_hot_count = self.re_clone_hot.find_iter(output).count();
        score -= (clone_hot_count as f64 * 0.03).min(0.3);

        // Penalty: deep nesting (4+ indent levels ≈ 16+ leading spaces or 4+ tabs)
        let code_lines: Vec<&str> = lines
            .iter()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with("//"))
            .copied()
            .collect();
        if !code_lines.is_empty() {
            let deep_count = code_lines
                .iter()
                .filter(|l| {
                    let leading_spaces = l.len() - l.trim_start().len();
                    leading_spaces >= 16 || l.starts_with("\t\t\t\t")
                })
                .count();
            let ratio = deep_count as f64 / code_lines.len() as f64;
            if ratio > 0.10 {
                score -= 0.2;
            }
        }

        // Penalty: functions >50 lines
        let mut fn_line_counts = Vec::new();
        let mut brace_depth: i32 = 0;
        let mut in_fn = false;
        let mut fn_lines = 0_usize;
        for line in &lines {
            if !in_fn && self.re_long_fn.is_match(line) && line.contains('{') {
                in_fn = true;
                brace_depth = 0;
            }
            if in_fn {
                fn_lines += 1;
                brace_depth += line.matches('{').count() as i32;
                brace_depth -= line.matches('}').count() as i32;
                if brace_depth <= 0 {
                    fn_line_counts.push(fn_lines);
                    in_fn = false;
                    fn_lines = 0;
                }
            }
        }
        let long_fns = fn_line_counts.iter().filter(|&&c| c > 50).count();
        score -= (long_fns as f64 * 0.05).min(0.3);

        // Penalty: magic numbers
        let mut magic_count = 0_usize;
        for line in &lines {
            let trimmed = line.trim();
            // Skip const/let/static lines, comments, attributes, test code
            if trimmed.starts_with("//")
                || trimmed.starts_with("const ")
                || trimmed.starts_with("let ")
                || trimmed.starts_with("static ")
                || trimmed.starts_with("#[")
                || trimmed.starts_with("pub const ")
            {
                continue;
            }
            for cap in self.re_magic_number.captures_iter(line) {
                if let Some(m) = cap.get(1) {
                    let val: i64 = m.as_str().parse().unwrap_or(0);
                    if val > 1 {
                        magic_count += 1;
                    }
                }
            }
        }
        score -= (magic_count as f64 * 0.02).min(0.2);

        score.max(0.0)
    }

    // ── Completeness (0.25) ──────────────────────────────────────────

    fn score_completeness(&self, output: &str) -> f64 {
        let mut score: f64 = 1.0;

        // TODO / FIXME / HACK / XXX
        let todo_count = self.re_todo_markers.find_iter(output).count();
        score -= (todo_count as f64 * 0.08).min(0.8);

        // unimplemented!() / todo!()
        let unimpl_count = self.re_unimplemented.find_iter(output).count();
        score -= (unimpl_count as f64 * 0.15).min(0.9);

        // stub / placeholder / not yet implemented
        let stub_count = self.re_stub_markers.find_iter(output).count();
        score -= (stub_count as f64 * 0.10).min(0.5);

        // Empty function bodies
        let empty_fn_count = self.re_empty_fn.find_iter(output).count();
        score -= (empty_fn_count as f64 * 0.15).min(0.6);

        // panic!() not in test code — simple heuristic: split on #[cfg(test)]
        let non_test_section = output.split("#[cfg(test)]").next().unwrap_or(output);
        let panic_count = self.re_panic_non_test.find_iter(non_test_section).count();
        score -= (panic_count as f64 * 0.10).min(0.3);

        score.max(0.0)
    }

    // ── Structural Integrity (0.20) ──────────────────────────────────

    fn score_structural_integrity(&self, output: &str) -> f64 {
        let mut score: f64 = 1.0;

        // Orphaned use statements — check if last path segment is referenced elsewhere
        let mut orphan_count = 0_usize;
        for m in self.re_use_stmt.find_iter(output) {
            let use_line = m.as_str();
            // Extract the last segment before the semicolon
            let clean = use_line.trim_end_matches(';').trim();
            if let Some(last_seg) = clean.rsplit("::").next() {
                let seg = last_seg
                    .trim_start_matches('{')
                    .trim_end_matches('}')
                    .trim();
                // For grouped imports, take just the first item
                let ident = seg.split(',').next().unwrap_or("").trim();
                if !ident.is_empty() && ident.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    // Count occurrences outside use lines
                    let rest: String = output
                        .lines()
                        .filter(|l| !l.trim_start().starts_with("use "))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !rest.contains(ident) {
                        orphan_count += 1;
                    }
                }
            }
        }
        score -= (orphan_count as f64 * 0.05).min(0.3);

        // #[allow(dead_code)]
        let dead_code_count = self.re_allow_dead.find_iter(output).count();
        score -= (dead_code_count as f64 * 0.10).min(0.3);

        // Wire markers
        let wire_count = self.re_wire_marker.find_iter(output).count();
        score -= (wire_count as f64 * 0.15).min(0.3);

        // No mod declarations in files >100 lines
        let line_count = output.lines().count();
        if line_count > 100 && !self.re_mod_decl.is_match(output) {
            score -= 0.1;
        }

        score.max(0.0)
    }

    // ── Documentation (0.15) ─────────────────────────────────────────

    fn score_documentation(&self, output: &str) -> f64 {
        let line_count = output.lines().count();
        if line_count < 10 {
            return 0.0;
        }

        let mut score: f64 = 0.0;

        // /// doc comments
        let doc_comments = self.re_doc_comment.find_iter(output).count();
        score += (doc_comments as f64 * 0.1).min(0.5);

        // //! module docs
        let module_docs = self.re_module_doc.find_iter(output).count();
        score += (module_docs as f64 * 0.2).min(0.4);

        // #[doc = ...]
        let doc_attrs = self.re_doc_attr.find_iter(output).count();
        score += (doc_attrs as f64 * 0.1).min(0.3);

        // Ratio of documented public items
        let pub_items = self.re_pub_item.find_iter(output).count();
        if pub_items > 0 {
            // Count pub items preceded by a doc comment (within 3 lines above)
            let lines: Vec<&str> = output.lines().collect();
            let mut documented = 0_usize;
            for (i, line) in lines.iter().enumerate() {
                if self.re_pub_item.is_match(line) {
                    let start = i.saturating_sub(3);
                    let preceding = &lines[start..i];
                    if preceding
                        .iter()
                        .any(|l| self.re_doc_comment.is_match(l) || self.re_module_doc.is_match(l))
                    {
                        documented += 1;
                    }
                }
            }
            let ratio = documented as f64 / pub_items as f64;
            score += ratio * 0.3;
        }

        score.min(1.0)
    }

    // ── Test Coverage (0.10) ─────────────────────────────────────────

    fn score_test_coverage(&self, output: &str) -> f64 {
        let mut score: f64 = 0.0;

        let test_attr_count = self.re_test_attr.find_iter(output).count();
        if test_attr_count == 0
            && !self.re_cfg_test.is_match(output)
            && self.re_assert.find_iter(output).count() == 0
            && self.re_test_fn_name.find_iter(output).count() == 0
        {
            return 0.0;
        }

        // #[test] count
        score += (test_attr_count as f64 * 0.1).min(0.4);

        // #[cfg(test)] presence
        if self.re_cfg_test.is_match(output) {
            score += 0.2;
        }

        // Assertions
        let assert_count = self.re_assert.find_iter(output).count();
        score += (assert_count as f64 * 0.02).min(0.3);

        // test_ prefixed function names
        let test_fn_count = self.re_test_fn_name.find_iter(output).count();
        score += (test_fn_count as f64 * 0.05).min(0.1);

        score.min(1.0)
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn calc() -> CodingQualityCalculator {
        CodingQualityCalculator::new()
    }

    fn round3(v: f64) -> f64 {
        (v * 1000.0).round() / 1000.0
    }

    // ── 1. Perfect code ──────────────────────────────────────────────

    #[test]
    fn test_perfect_code() {
        let code = r#"
//! A well-documented module.

use std::collections::HashMap;

/// Represents a user with a name and score.
pub struct User {
    /// The user's display name.
    pub name: String,
    /// The user's current score.
    pub score: f64,
}

/// Creates a default user.
pub fn create_user(name: &str) -> User {
    User {
        name: name.to_string(),
        score: 0.0,
    }
}

/// Looks up a user by name.
pub fn lookup(map: &HashMap<String, User>, name: &str) -> Option<&User> {
    map.get(name)
}

pub mod helpers {
    /// Helper to format a score.
    pub fn format_score(val: f64) -> String {
        format!("{:.2}", val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_user() {
        let u = create_user("Alice");
        assert_eq!(u.name, "Alice");
        assert_eq!(u.score, 0.0);
    }

    #[test]
    fn test_lookup_found() {
        let mut map = HashMap::new();
        map.insert("Bob".into(), create_user("Bob"));
        assert!(lookup(&map, "Bob").is_some());
    }

    #[test]
    fn test_lookup_missing() {
        let map: HashMap<String, User> = HashMap::new();
        assert!(lookup(&map, "Nope").is_none());
    }

    #[test]
    fn test_format_score() {
        assert_eq!(helpers::format_score(3.14159), "3.14");
    }
}
"#;
        let b = calc().score(code);
        assert!(
            b.composite >= 0.85,
            "perfect code composite={}",
            b.composite
        );
        assert!(b.code_quality >= 0.5, "cq={}", b.code_quality);
        assert!(b.completeness >= 0.9, "comp={}", b.completeness);
        assert!(b.test_coverage >= 0.5, "tc={}", b.test_coverage);
    }

    // ── 2. Code with 3 TODOs ─────────────────────────────────────────

    #[test]
    fn test_three_todos() {
        let code = r#"
//! Module doc

/// Public fn
pub fn do_work() -> i32 {
    // TODO: implement real logic
    // TODO: handle edge case
    // TODO: optimize later
    let result = 0;
    result
}

pub mod inner {
    /// Inner helper
    pub fn helper() {}
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_placeholder() {
        assert_eq!(1, 1);
    }
}
"#;
        let b = calc().score(code);
        // 3 TODOs -> completeness penalized by 0.24
        assert!(b.completeness <= 0.80, "completeness={}", b.completeness);
        assert!(
            b.completeness >= 0.50,
            "completeness too low={}",
            b.completeness
        );
    }

    // ── 3. Code with no docs ─────────────────────────────────────────

    #[test]
    fn test_no_docs() {
        // 10+ lines required for documentation to score above 0.0
        let code = r#"
pub fn alpha() -> i32 { 1 }
pub fn beta() -> i32 { 2 }
pub fn gamma() -> i32 { 3 }
pub fn delta() -> i32 { 4 }
pub fn epsilon() -> i32 { 5 }
pub fn zeta() -> i32 { 6 }
pub fn eta() -> i32 { 7 }
pub fn theta() -> i32 { 8 }
pub fn iota() -> i32 { 9 }
pub fn kappa() -> i32 { 10 }
pub fn lambda() -> i32 { 11 }
"#;
        let b = calc().score(code);
        assert!(
            b.documentation <= 0.1,
            "documentation should be near zero, got={}",
            b.documentation
        );
    }

    // ── 4. Code with 5+ unwrap() ────────────────────────────────────

    #[test]
    fn test_many_unwraps() {
        let code = r#"
//! Module

/// Process data
pub fn process(data: &str) -> String {
    let a = data.parse::<i32>().unwrap();
    let b = data.parse::<i32>().unwrap();
    let c = data.parse::<i32>().unwrap();
    let d = data.parse::<i32>().unwrap();
    let e = data.parse::<i32>().unwrap();
    let f = data.parse::<i32>().unwrap();
    format!("{}", a + b + c + d + e + f)
}

pub mod utils {
    /// A util
    pub fn id(x: i32) -> i32 { x }
}
"#;
        let b = calc().score(code);
        // 6 unwraps -> penalty 0.30
        assert!(b.code_quality <= 0.75, "cq={}", b.code_quality);
    }

    // ── 5. Code with no tests ───────────────────────────────────────

    #[test]
    fn test_no_tests() {
        let code = r#"
//! A module without tests.

/// Add two numbers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Subtract.
pub fn sub(a: i32, b: i32) -> i32 {
    a - b
}

pub mod math {
    /// Multiply
    pub fn mul(a: i32, b: i32) -> i32 { a * b }
}
"#;
        let b = calc().score(code);
        assert_eq!(b.test_coverage, 0.0, "test_coverage={}", b.test_coverage);
    }

    // ── 6. Code with #[allow(dead_code)] ────────────────────────────

    #[test]
    fn test_allow_dead_code() {
        let code = r#"
//! Module

#[allow(dead_code)]
pub fn unused_a() {}

#[allow(dead_code)]
pub fn unused_b() {}

#[allow(dead_code)]
pub fn unused_c() {}

/// Used
pub fn used() -> i32 { 1 }

pub mod inner {
    /// X
    pub fn x() {}
}
"#;
        let b = calc().score(code);
        // 3 allow(dead_code) -> penalty 0.30
        assert!(
            b.structural_integrity <= 0.75,
            "si={}",
            b.structural_integrity
        );
    }

    // ── 7. Empty input ──────────────────────────────────────────────

    #[test]
    fn test_empty_input() {
        let b = calc().score("");
        assert_eq!(b.code_quality, 0.0);
        assert_eq!(b.completeness, 0.0);
        assert_eq!(b.structural_integrity, 0.0);
        assert_eq!(b.documentation, 0.0);
        assert_eq!(b.test_coverage, 0.0);
        assert_eq!(b.composite, 0.0);
    }

    // ── 8. Mixed quality ────────────────────────────────────────────

    #[test]
    fn test_mixed_quality() {
        let code = r#"
//! Module docs

use std::collections::HashMap;

/// A struct
pub struct Config {
    pub name: String,
}

pub fn process(data: &str) -> String {
    // TODO: validate input
    let val = data.parse::<i32>().unwrap();
    format!("{}", val)
}

pub mod helpers {
    /// A helper
    pub fn noop() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_process() {
        assert_eq!(process("42"), "42");
    }
}
"#;
        let b = calc().score(code);
        assert!(
            b.composite >= 0.45 && b.composite <= 0.85,
            "mixed composite={}",
            b.composite
        );
    }

    // ── 9. Minimal stub ─────────────────────────────────────────────

    #[test]
    fn test_minimal_stub() {
        let code = r#"
//! Stub module

/// Stub A
pub fn do_a() {
    todo!()
}

/// Stub B
pub fn do_b() {
    todo!()
}

/// Stub C
pub fn do_c() {
    todo!()
}

pub mod inner {
    /// Stub inner
    pub fn inner_fn() { todo!() }
}
"#;
        let b = calc().score(code);
        assert!(
            b.completeness <= 0.20,
            "completeness for stubs={}",
            b.completeness
        );
    }

    // ── 10. Well-documented but untested ────────────────────────────

    #[test]
    fn test_documented_no_tests() {
        let code = r#"
//! A thoroughly documented module with no tests.
//!
//! This module provides arithmetic operations.

/// Adds two integers and returns the result.
///
/// # Arguments
/// * `a` - First operand
/// * `b` - Second operand
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Multiplies two integers.
///
/// # Arguments
/// * `a` - First operand
/// * `b` - Second operand
pub fn mul(a: i32, b: i32) -> i32 {
    a * b
}

/// Divides `a` by `b`, returning `None` if `b` is zero.
pub fn safe_div(a: i32, b: i32) -> Option<i32> {
    if b == 0 {
        None
    } else {
        Some(a / b)
    }
}

pub mod extras {
    /// Negate a value.
    pub fn neg(x: i32) -> i32 { -x }
}
"#;
        let b = calc().score(code);
        assert!(b.documentation >= 0.5, "docs={}", b.documentation);
        assert_eq!(b.test_coverage, 0.0, "test_coverage={}", b.test_coverage);
    }

    // ── Phase threshold tests ───────────────────────────────────────

    #[test]
    fn test_phase_thresholds() {
        assert_eq!(phase_threshold(1), 0.75);
        assert_eq!(phase_threshold(2), 0.80);
        assert_eq!(phase_threshold(3), 0.82);
        assert_eq!(phase_threshold(4), 0.85);
        assert_eq!(phase_threshold(5), 0.88);
        assert_eq!(phase_threshold(6), 0.95);
        assert_eq!(phase_threshold(99), 0.80);
    }

    // ── meets_threshold ─────────────────────────────────────────────

    #[test]
    fn test_meets_threshold() {
        let c = calc();
        assert!(!c.meets_threshold("", 1));
    }

    // ── Composite rounding ──────────────────────────────────────────

    #[test]
    fn test_composite_rounding() {
        let b = calc().score("fn foo() { 1 }");
        // composite should be rounded to 3 decimal places
        assert_eq!(b.composite, round3(b.composite));
    }
}
