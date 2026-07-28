use super::*;

fn calc() -> CodingQualityCalculator {
    CodingQualityCalculator::new()
}

fn rust(code: &str) -> String {
    format!("```rust\n{code}\n```")
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

// ── 1. Perfect code ──────────────────────────────────────────────────

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
    let b = calc().score(&rust(code));
    assert!(
        b.composite >= 0.85,
        "perfect code composite={}",
        b.composite
    );
    assert!(b.code_quality >= 0.5, "cq={}", b.code_quality);
    assert!(b.completeness >= 0.9, "comp={}", b.completeness);
    assert!(b.test_coverage >= 0.5, "tc={}", b.test_coverage);
}

// ── 2. Code with 3 TODOs ─────────────────────────────────────────────

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
    let b = calc().score(&rust(code));
    assert!(b.completeness <= 0.8, "completeness={}", b.completeness);
    assert!(
        b.completeness >= 0.50,
        "completeness too low={}",
        b.completeness
    );
}

// ── 3. Code with no docs ─────────────────────────────────────────────

#[test]
fn test_no_docs() {
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
    let b = calc().score(&rust(code));
    assert!(
        b.documentation <= 0.1,
        "documentation should be near zero, got={}",
        b.documentation
    );
}

// ── 4. Code with 5+ unwrap() ─────────────────────────────────────────

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
    let b = calc().score(&rust(code));
    assert!(b.code_quality <= 0.75, "cq={}", b.code_quality);
}

// ── 5. Code with no tests ────────────────────────────────────────────

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
    let b = calc().score(&rust(code));
    assert_eq!(b.test_coverage, 0.0, "test_coverage={}", b.test_coverage);
}

// ── 6. Code with #[allow(dead_code)] ─────────────────────────────────

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
    let b = calc().score(&rust(code));
    assert!(
        b.structural_integrity <= 0.75,
        "si={}",
        b.structural_integrity
    );
}

// ── 7. Empty input ───────────────────────────────────────────────────

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

// ── 8. Mixed quality ─────────────────────────────────────────────────

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
    let b = calc().score(&rust(code));
    assert!(
        b.composite >= 0.45 && b.composite <= 0.85,
        "mixed composite={}",
        b.composite
    );
}

// ── 9. Minimal stub ──────────────────────────────────────────────────

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
    let b = calc().score(&rust(code));
    assert!(
        b.completeness <= 0.20,
        "completeness for stubs={}",
        b.completeness
    );
}

// ── 10. Well-documented but untested ─────────────────────────────────

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
    let b = calc().score(&rust(code));
    assert!(b.documentation >= 0.5, "docs={}", b.documentation);
    assert_eq!(b.test_coverage, 0.0, "test_coverage={}", b.test_coverage);
}

// ── Phase threshold tests ─────────────────────────────────────────────

#[test]
fn test_phase_thresholds() {
    assert_eq!(phase_threshold(1), 0.75);
    assert_eq!(phase_threshold(2), 0.8);
    assert_eq!(phase_threshold(3), 0.82);
    assert_eq!(phase_threshold(4), 0.85);
    assert_eq!(phase_threshold(5), 0.88);
    assert_eq!(phase_threshold(6), 0.95);
    assert_eq!(phase_threshold(99), 0.8);
}

// ── meets_threshold ──────────────────────────────────────────────────

#[test]
fn test_meets_threshold() {
    let c = calc();
    assert!(!c.meets_threshold("", 1));
}

// ── Composite rounding ───────────────────────────────────────────────

#[test]
fn test_composite_rounding() {
    let b = calc().score(&rust("fn foo() { 1 }"));
    assert_eq!(b.composite, round3(b.composite));
}
