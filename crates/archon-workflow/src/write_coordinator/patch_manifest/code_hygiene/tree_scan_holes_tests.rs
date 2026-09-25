//! Fail-open holes a hostile review found in the syntax-tree reading: the
//! gate must never be weaker than the hand scanner was.

use super::*;

fn names(path: &str, code: &str) -> Vec<(String, usize)> {
    scan_functions(path, code)
        .functions
        .into_iter()
        .map(|function| (function.name, function.line))
        .collect()
}

fn named(name: &str, line: usize) -> (String, usize) {
    (name.to_string(), line)
}

fn ifs(count: usize) -> String {
    (0..count)
        .map(|idx| format!("  if (c{idx}) {{}}\n"))
        .collect()
}

fn rust_ifs(count: usize) -> String {
    (0..count)
        .map(|idx| format!("        if c{idx} {{}}\n"))
        .collect()
}

#[test]
fn unnamed_and_context_named_script_functions_are_found() {
    let code = "module.exports = function () { if (a) {} };\n\
                exports.h = async () => { if (a) {} };\n\
                const o = { h: () => { if (a) {} } };\n\
                Foo.prototype.bar = function () { if (a) {} };\n\
                const M = memo(function C() { if (a) {} });\n\
                app.get('/', (req, res) => { if (a) {} });\n\
                setTimeout(() => { if (a) {} }, 1);\n";
    assert_eq!(
        names("a.js", code),
        vec![
            named("module.exports", 1),
            named("exports.h", 2),
            named("h", 3),
            named("Foo.prototype.bar", 4),
            named("C", 5),
            named("app.get('/')", 6),
            named("setTimeout(...)", 7),
        ]
    );
    assert_eq!(
        names("a.js", "export default function () { if (a) {} }\n"),
        vec![named("default", 1)]
    );
    assert_eq!(
        names("a.ts", "export default () => { if (a) {} };\n"),
        vec![named("default", 1)]
    );
    assert_eq!(
        names("a.go", "package p\nvar H = func() {\n\tif a {}\n}\n"),
        vec![named("H", 2)]
    );
}

#[test]
fn an_over_cap_exported_function_expression_is_rejected() {
    let code = format!("module.exports = function () {{\n{}}};\n", ifs(20));
    let err = validate_complexity("src/a.js", None, &code, 15).expect_err("judged");
    assert!(err.to_string().contains("'module.exports'"), "{err}");
}

#[test]
fn rust_functions_inside_macros_are_judged() {
    let code = format!(
        "proptest! {{\n    #[test]\n    fn prop(x in 0..10) {{\n{}    }}\n}}\n",
        rust_ifs(20)
    );
    let err = validate_complexity("src/a.rs", None, &code, 15).expect_err("judged");
    assert!(
        err.to_string().contains("function 'prop' at line 3"),
        "{err}"
    );
    let cfg = format!(
        "cfg_if::cfg_if! {{\n    if #[cfg(unix)] {{\n        fn native() {{\n{}        }}\n    }}\n}}\n",
        rust_ifs(20)
    );
    let err = validate_complexity("src/a.rs", None, &cfg, 15).expect_err("judged");
    assert!(
        err.to_string().contains("function 'native' at line 3"),
        "{err}"
    );
}

#[test]
fn a_syntax_error_does_not_exempt_a_new_over_cap_function() {
    let code = format!("fn broken() {{\n    let y = ;\n{}}}\n", rust_ifs(20));
    let err = validate_complexity("src/a.rs", None, &code, 15).expect_err("judged");
    let text = err.to_string();
    assert!(
        text.contains("'broken'") && text.contains("syntax error"),
        "{text}"
    );
    // Against a baseline counterpart that already scored as much, it passes.
    let baseline = format!("fn broken() {{\n{}}}\n", rust_ifs(20));
    let notes = validate_complexity("src/a.rs", Some(&baseline), &code, 15).expect("no worse");
    assert!(!notes.is_empty());
}

#[test]
fn valid_code_the_grammar_rejects_does_not_disable_the_baseline() {
    let prefix = "unsafe extern \"C\" {\n    safe fn ext();\n}\n";
    let heavy = |branches| format!("fn heavy() {{\n{}}}\n", rust_ifs(branches));
    let baseline = format!("{prefix}{}", heavy(20));
    let same = format!("{prefix}{}", heavy(20));
    assert!(validate_complexity("src/a.rs", Some(&baseline), &same, 15).is_ok());
    let grown = format!("{prefix}{}", heavy(21));
    let err = validate_complexity("src/a.rs", Some(&baseline), &grown, 15).expect_err("grew");
    assert!(err.to_string().contains("was 21, now 22"), "{err}");
    let added = format!("{prefix}{}fn fresh() {{\n{}}}\n", heavy(20), rust_ifs(20));
    let err = validate_complexity("src/a.rs", Some(&baseline), &added, 15).expect_err("new");
    assert!(err.to_string().contains("'fresh'"), "{err}");
}

#[test]
fn an_unreadable_baseline_function_only_excuses_its_own_signature() {
    let baseline = "fn a(x: u8) {\n    let q = ;\n}\n";
    let post = format!("fn a(y: u16) {{\n{}}}\n", rust_ifs(20));
    let err = validate_complexity("src/a.rs", Some(baseline), &post, 15).expect_err("new");
    assert!(err.to_string().contains("'a'"), "{err}");
}

#[test]
fn an_unpairable_baseline_excuses_only_names_it_contains() {
    // Stray tree error AND a hand-scanner sync loss: neither baseline reading
    // is complete, so a post function is new unless its name is in the text.
    let heavy = format!("fn heavy() {{\n{}}}\n", rust_ifs(20));
    let baseline = format!("{heavy})\nfn broken() {{\n");
    let post = format!("{heavy}fn fresh() {{\n{}}}\n", rust_ifs(20));
    let err = validate_complexity("src/a.rs", Some(&baseline), &post, 15).expect_err("new");
    assert!(err.to_string().contains("'fresh'"), "{err}");
    assert!(validate_complexity("src/a.rs", Some(&baseline), &heavy, 15).is_ok());
}
