//! Second-review cases: shell `#`, single-line literals left open, backtick
//! escapes, and a scan that loses sync.

use super::*;

fn spans(path: &str, code: &str) -> Vec<(String, usize, u32)> {
    function_scores(path, code)
        .into_iter()
        .map(|function| (function.name, function.line, function.score))
        .collect()
}

fn span(name: &str, line: usize, score: u32) -> (String, usize, u32) {
    (name.to_string(), line, score)
}

/// `name` scoring `1 + branches` in `ext`'s brace syntax.
fn heavy(name: &str, branches: usize) -> String {
    let body: String = (0..branches)
        .map(|idx| format!("    if (c{idx}) {{}}\n"))
        .collect();
    format!("function {name}() {{\n{body}}}\n")
}

#[test]
fn shell_parameter_expansions_are_not_comments() {
    // The forms the repository's own scripts use, unquoted.
    let code = "count() {\n\
                \x20 [[ ${#present[@]} -gt 0 ]] || return 0\n\
                \x20 marker=${hit##*:}; rel=${f#./}; rest=${line#*:}\n\
                \x20 echo $# && echo done # trailing { comment\n\
                \x20 x=1;# comment after ; {\n\
                }\n\
                other() {\n\
                \x20 if true; then :; fi\n\
                }\n";
    assert_eq!(
        spans("a.sh", code),
        vec![span("count", 1, 3), span("other", 7, 2)]
    );
}

#[test]
fn an_unclosed_single_line_quote_is_rescanned_as_code() {
    let tsx = "function Msg() {\n  return <p>Couldn't {a && b}</p>;\n}\n";
    assert_eq!(spans("a.tsx", tsx), vec![span("Msg", 1, 2)]);
    let regex = "function f(s) {\n  return s.replace(/\"/g, '') && s.length > 0;\n}\n";
    assert_eq!(spans("a.js", regex), vec![span("f", 1, 2)]);
    let digits = "int f() {\n  int n = 1'000; if (n && m) {}\n}\n";
    assert_eq!(spans("a.cpp", digits), vec![span("f", 1, 3)]);
}

#[test]
fn backtick_escapes_apply_except_in_go() {
    let js = "function f() {\n  const t = `a\\`{`;\n  if (a) {}\n}\nfunction g() {}\n";
    assert_eq!(spans("a.js", js), vec![span("f", 1, 2), span("g", 5, 1)]);
    let go = "func f() {\n\ts := `C:\\`\n\tif a {}\n}\nfunc g() {}\n";
    assert_eq!(spans("a.go", go), vec![span("f", 1, 2), span("g", 5, 1)]);
}

#[test]
fn rust_raw_c_strings_are_literals() {
    let code = "fn a() {\n    let s = cr#\"say \" { if\"#;\n    let t = cr\"}\";\n    if b {}\n}\nfn c() {}\n";
    assert_eq!(spans("a.rs", code), vec![span("a", 1, 2), span("c", 6, 1)]);
}

#[test]
fn a_file_that_loses_sync_is_not_judged() {
    // `broken` never closes: the scanner lost sync, so nothing in the file
    // is judged, even the over-cap function that closed before it.
    let text = format!("{}function broken() {{\n  if (x) {{\n", heavy("done", 20));
    assert!(validate_complexity("src/a.js", None, &text, 15).is_ok());
    // A baseline that loses sync makes the post-patch file unjudgeable too.
    let baseline = format!("{}function broken() {{\n", heavy("done", 20));
    let post = heavy("done", 21);
    assert!(validate_complexity("src/a.js", Some(&baseline), &post, 15).is_ok());
}

#[test]
fn literal_text_in_signatures_keeps_same_named_functions_apart() {
    // Emptying literals made these two signatures identical, so swapping the
    // two functions paired each with the other's baseline score.
    let x = heavy("f", 19).replace("f()", "f(a = \"x\")");
    let y = heavy("f", 16).replace("f()", "f(a = \"y\")");
    let baseline = format!("{x}{y}");
    let swapped = format!("{y}{x}");
    assert!(validate_complexity("src/a.js", Some(&baseline), &swapped, 15).is_ok());
}

#[test]
fn identical_signatures_pair_equal_scores_before_order() {
    let baseline = format!("{}{}", heavy("make", 19), heavy("make", 16));
    let swapped = format!("{}{}", heavy("make", 16), heavy("make", 19));
    assert!(validate_complexity("src/a.js", Some(&baseline), &swapped, 15).is_ok());
    let both_changed = format!("{}{}", heavy("make", 17), heavy("make", 18));
    assert!(validate_complexity("src/a.js", Some(&baseline), &both_changed, 15).is_ok());
    let grown = format!("{}{}", heavy("make", 20), heavy("make", 16));
    let err = validate_complexity("src/a.js", Some(&baseline), &grown, 15).expect_err("grew");
    assert!(err.to_string().contains("was 20, now 21"), "{err}");
    let extra = format!(
        "{}{}{}",
        heavy("make", 19),
        heavy("make", 16),
        heavy("make", 17)
    );
    let err = validate_complexity("src/a.js", Some(&baseline), &extra, 15).expect_err("extra");
    assert!(
        matches!(err, PatchError::FunctionTooComplex { complexity: 18, .. }),
        "{err:?}"
    );
}
