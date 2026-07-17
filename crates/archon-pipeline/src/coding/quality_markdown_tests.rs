use super::quality::{CodingQualityCalculator, QualityBreakdown};

fn assert_zero(breakdown: &QualityBreakdown) {
    assert_eq!(breakdown.code_quality, 0.0);
    assert_eq!(breakdown.completeness, 0.0);
    assert_eq!(breakdown.structural_integrity, 0.0);
    assert_eq!(breakdown.documentation, 0.0);
    assert_eq!(breakdown.test_coverage, 0.0);
    assert_eq!(breakdown.composite, 0.0);
}

#[test]
fn prose_does_not_change_rust_fence_score() {
    let calculator = CodingQualityCalculator::new();
    let rust = "```rust\nfn answer() -> i32 { 42 }\n```";
    let with_prose =
        format!("TODO: this prose describes a stub and panic! with number 99.\n\n{rust}");

    let expected = calculator.score(rust);
    let actual = calculator.score(&with_prose);

    assert_eq!(actual.code_quality, expected.code_quality);
    assert_eq!(actual.completeness, expected.completeness);
    assert_eq!(actual.structural_integrity, expected.structural_integrity);
    assert_eq!(actual.documentation, expected.documentation);
    assert_eq!(actual.test_coverage, expected.test_coverage);
    assert_eq!(actual.composite, expected.composite);
}

#[test]
fn unsupported_fences_receive_no_rust_score() {
    let calculator = CodingQualityCalculator::new();
    let output = r#"
```python
# TODO
#[test]
/// documentation
assert_eq!(1, 1)
```
```javascript
panic!("not Rust");
```
"#;

    assert_zero(&calculator.score(output));
}

#[test]
fn prose_only_and_untagged_fences_receive_no_rust_score() {
    let calculator = CodingQualityCalculator::new();

    assert_zero(&calculator.score("TODO: explain the implementation with assert_eq!"));
    assert_zero(&calculator.score("```\nfn looks_like_rust() {}\n```"));
}

#[test]
fn unsupported_fences_do_not_change_rust_fence_score() {
    let calculator = CodingQualityCalculator::new();
    let rust = "```rust\nfn answer() -> i32 { 42 }\n```";
    let mixed = format!(
        "```python\n# TODO stub\nassert_eq!(1, 1)\n```\n{rust}\n```javascript\npanic!(\"x\")\n```"
    );

    let expected = calculator.score(rust);
    let actual = calculator.score(&mixed);

    assert_eq!(actual.composite, expected.composite);
    assert_eq!(actual.completeness, expected.completeness);
    assert_eq!(actual.test_coverage, expected.test_coverage);
}

#[test]
fn rust_and_rs_fence_tags_are_case_insensitive() {
    let calculator = CodingQualityCalculator::new();
    let body = "fn answer() -> i32 { 42 }";
    let rust = calculator.score(&format!("```rust\n{body}\n```"));
    let uppercase = calculator.score(&format!("```Rust\n{body}\n```"));
    let rs = calculator.score(&format!("```rs\n{body}\n```"));

    assert_eq!(uppercase.composite, rust.composite);
    assert_eq!(rs.composite, rust.composite);
}

#[test]
fn rust_fence_still_receives_rust_specific_penalties() {
    let calculator = CodingQualityCalculator::new();
    let clean = calculator.score("```rust\nfn value() -> i32 { 1 }\n```");
    let incomplete = calculator.score(
        "```rust\nfn value(input: &str) -> i32 {\n    // TODO: remove unwrap\n    input.parse().unwrap()\n}\n```",
    );

    assert!(incomplete.completeness < clean.completeness);
    assert!(incomplete.code_quality < clean.code_quality);
}

#[test]
fn rust_tag_inside_unsupported_fence_is_ignored() {
    let calculator = CodingQualityCalculator::new();
    let output = "```python\n```rust\nfn should_not_be_scored() {}\n```\n```\n";

    assert_zero(&calculator.score(output));
}

#[test]
fn four_backtick_rust_fence_is_scored() {
    let calculator = CodingQualityCalculator::new();
    let triple = calculator.score("```rust\nfn value() -> i32 { 1 }\n```");
    let quadruple = calculator.score("````rust\nfn value() -> i32 { 1 }\n````");

    assert_eq!(quadruple.composite, triple.composite);
}

#[test]
fn four_space_indented_pseudo_fence_is_ignored() {
    let calculator = CodingQualityCalculator::new();

    assert_zero(&calculator.score("    ```rust\n    fn value() -> i32 { 1 }\n    ```"));
}

#[test]
fn shorter_or_annotated_backtick_lines_do_not_close_longer_fence() {
    let calculator = CodingQualityCalculator::new();
    let output = "````rust\nfn value() -> i32 {\n```\n```not-a-closer\n// TODO\n1\n}\n````";
    let breakdown = calculator.score(output);

    assert!(breakdown.composite > 0.0);
    assert!(breakdown.completeness < 1.0);
}

#[test]
fn non_ascii_whitespace_does_not_close_rust_fence() {
    let calculator = CodingQualityCalculator::new();
    let output = "```rust\nfn value() -> i32 {\n```\u{00a0}\n// TODO\n1\n}\n```";
    let breakdown = calculator.score(output);

    assert!(breakdown.composite > 0.0);
    assert!(breakdown.completeness < 1.0);
}

#[test]
fn tilde_rust_fence_is_scored() {
    let calculator = CodingQualityCalculator::new();
    let backticks = calculator.score("```rust\nfn value() -> i32 { 1 }\n```");
    let tildes = calculator.score("~~~rs\nfn value() -> i32 { 1 }\n~~~");

    assert_eq!(tildes.composite, backticks.composite);
}

#[test]
fn unsupported_only_output_never_meets_phase_threshold() {
    let calculator = CodingQualityCalculator::new();

    assert!(!calculator.meets_threshold("```python\nprint('complete')\n```", 1));
}
