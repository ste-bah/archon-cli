use super::*;

const MAX: u32 = 15;

/// `fn name` scoring `1 + branches`.
fn function(name: &str, branches: usize) -> String {
    let mut code = format!("fn {name}(\n    a: u8,\n) -> u8 {{\n");
    for idx in 0..branches {
        code.push_str(&format!("    if c{idx} {{}}\n"));
    }
    code.push_str("    0\n}\n");
    code
}

fn file(functions: &[(&str, usize)]) -> String {
    functions
        .iter()
        .map(|(name, branches)| function(name, *branches))
        .collect()
}

#[test]
fn unchanged_over_cap_function_passes() {
    let baseline = file(&[("heavy", 19), ("light", 1)]);
    let text = file(&[("heavy", 19), ("light", 2), ("added", 3)]);
    assert!(validate_complexity("src/a.rs", Some(&baseline), &text, MAX).is_ok());
}

#[test]
fn reduced_but_still_over_cap_function_passes() {
    let baseline = file(&[("heavy", 19)]);
    let text = file(&[("heavy", 17)]);
    assert!(validate_complexity("src/a.rs", Some(&baseline), &text, MAX).is_ok());
}

#[test]
fn increased_over_cap_function_is_rejected_with_was_and_now() {
    let baseline = file(&[("light", 1), ("heavy", 19)]);
    let text = file(&[("light", 1), ("heavy", 20)]);
    let err = validate_complexity("src/a.rs", Some(&baseline), &text, MAX).expect_err("worse");
    match &err {
        PatchError::FunctionComplexityIncreased {
            function,
            line,
            baseline,
            complexity,
            ..
        } => assert_eq!(
            (function.as_str(), *line, *baseline, *complexity),
            ("heavy", 7, 20, 21)
        ),
        other => panic!("expected FunctionComplexityIncreased, got {other:?}"),
    }
    let text = err.to_string();
    assert!(text.contains("was 20, now 21"), "{text}");
    assert!(
        text.contains("function 'heavy'") && text.contains("exceeds max 15"),
        "{text}"
    );
}

#[test]
fn under_cap_function_grown_over_cap_is_rejected_with_was_and_now() {
    let baseline = file(&[("grows", 10)]);
    let text = file(&[("grows", 15)]);
    let err = validate_complexity("src/a.rs", Some(&baseline), &text, MAX).expect_err("over");
    assert!(err.to_string().contains("was 11, now 16"), "{err}");
}

#[test]
fn new_over_cap_function_in_existing_file_is_rejected() {
    let baseline = file(&[("heavy", 19)]);
    let text = file(&[("heavy", 19), ("fresh", 16)]);
    let err = validate_complexity("src/a.rs", Some(&baseline), &text, MAX).expect_err("new");
    assert!(
        matches!(&err, PatchError::FunctionTooComplex { function, complexity: 17, .. } if function == "fresh"),
        "{err:?}"
    );
}

#[test]
fn renamed_over_cap_function_counts_as_new() {
    let baseline = file(&[("heavy", 19)]);
    let text = file(&[("heavier", 19)]);
    let err = validate_complexity("src/a.rs", Some(&baseline), &text, MAX).expect_err("renamed");
    assert!(
        matches!(&err, PatchError::FunctionTooComplex { function, .. } if function == "heavier"),
        "{err:?}"
    );
}

#[test]
fn duplicate_names_pair_up_in_file_order() {
    // Two `new` functions (separate `impl` blocks): the second one grew.
    let baseline = file(&[("new", 19), ("new", 16)]);
    let unchanged = file(&[("new", 19), ("new", 16)]);
    assert!(validate_complexity("src/a.rs", Some(&baseline), &unchanged, MAX).is_ok());
    let grown = file(&[("new", 19), ("new", 17)]);
    let err = validate_complexity("src/a.rs", Some(&baseline), &grown, MAX).expect_err("grew");
    assert!(err.to_string().contains("was 17, now 18"), "{err}");
    // A third `new` has no baseline counterpart: judged as new.
    let added = file(&[("new", 19), ("new", 16), ("new", 16)]);
    let err = validate_complexity("src/a.rs", Some(&baseline), &added, MAX).expect_err("added");
    assert!(
        matches!(err, PatchError::FunctionTooComplex { .. }),
        "{err:?}"
    );
}

#[test]
fn new_file_judges_every_function() {
    let text = file(&[("heavy", 19)]);
    let err = validate_complexity("src/a.rs", None, &text, MAX).expect_err("new file");
    assert!(
        matches!(err, PatchError::FunctionTooComplex { .. }),
        "{err:?}"
    );
}

/// `header` opening a body that scores `1 + branches`.
fn with_header(header: &str, branches: usize) -> String {
    let body: String = (0..branches)
        .map(|idx| format!("    if c{idx} {{}}\n"))
        .collect();
    format!("{header} {{\n{body}}}\n")
}

#[test]
fn same_named_function_added_above_an_over_cap_one_does_not_shift_pairing() {
    let heavy = with_header("fn new(a: u8) -> Self", 19);
    let baseline = format!("impl B {{\n{heavy}}}\n");
    let text = format!(
        "impl A {{\n{}}}\nimpl B {{\n{heavy}}}\n",
        with_header("fn new() -> Self", 0)
    );
    assert!(validate_complexity("src/a.rs", Some(&baseline), &text, MAX).is_ok());
}

#[test]
fn same_named_function_removed_above_an_over_cap_one_does_not_shift_pairing() {
    let heavy = with_header("fn new(a: u8) -> Self", 19);
    let baseline = format!("{}{heavy}", with_header("fn new() -> Self", 0));
    assert!(validate_complexity("src/a.rs", Some(&baseline), &heavy, MAX).is_ok());
}

#[test]
fn changed_signature_beside_an_added_duplicate_is_judged_against_the_baseline_max() {
    let baseline = with_header("fn new(a: u8) -> Self", 19);
    let same = format!(
        "{}{}",
        with_header("fn new() -> Self", 0),
        with_header("fn new(a: u8, b: u8) -> Self", 19)
    );
    assert!(validate_complexity("src/a.rs", Some(&baseline), &same, MAX).is_ok());
    let grown = format!(
        "{}{}",
        with_header("fn new() -> Self", 0),
        with_header("fn new(a: u8, b: u8) -> Self", 20)
    );
    let err = validate_complexity("src/a.rs", Some(&baseline), &grown, MAX).expect_err("grew");
    assert!(err.to_string().contains("was 20, now 21"), "{err}");
}

#[test]
fn new_over_cap_duplicate_beside_an_unchanged_one_is_rejected_as_new() {
    let heavy = with_header("fn new(a: u8) -> Self", 19);
    let text = format!("{heavy}{}", with_header("fn new(b: u16) -> Self", 16));
    let err = validate_complexity("src/a.rs", Some(&heavy), &text, MAX).expect_err("new");
    assert!(
        matches!(&err, PatchError::FunctionTooComplex { line: 22, .. }),
        "{err:?}"
    );
}

#[test]
fn changed_signature_of_a_unique_function_still_ratchets() {
    let baseline = with_header("fn load(a: u8) -> u8", 19);
    let same = with_header("fn load(\n    a: u8,\n    b: u8,\n) -> u8", 19);
    assert!(validate_complexity("src/a.rs", Some(&baseline), &same, MAX).is_ok());
    let grown = with_header("fn load(a: u8, b: u8) -> u8", 20);
    let err = validate_complexity("src/a.rs", Some(&baseline), &grown, MAX).expect_err("grew");
    assert!(err.to_string().contains("was 20, now 21"), "{err}");
}
