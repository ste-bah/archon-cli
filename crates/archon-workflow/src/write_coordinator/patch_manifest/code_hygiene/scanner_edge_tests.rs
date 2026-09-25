//! Scanner cases a hostile review found: literals, Go receivers, EOF,
//! callbacks, brace-on-next-line, attributes and raw identifiers.

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

#[test]
fn go_receiver_methods_are_named_after_the_method() {
    let code = "func (s *Server) Handle(w Writer) {\n\tif x {}\n}\n\
                func Map[T any](xs []T) {\n}\n\
                func plain(a int) {\n}\n";
    assert_eq!(
        spans("a.go", code),
        vec![span("Handle", 1, 2), span("Map", 4, 1), span("plain", 6, 1)]
    );
}

#[test]
fn format_string_braces_and_hash_do_not_desync_later_functions() {
    let code = "fn a() {\n    println!(\"{:#?}\", x);\n    if b {}\n}\n\
                fn c() {\n    if d {}\n}\n";
    assert_eq!(spans("a.rs", code), vec![span("a", 1, 2), span("c", 5, 2)]);
}

#[test]
fn string_and_char_literals_hide_braces_and_keywords() {
    let code = "fn a<'a>(x: &'a str) {\n\
                \x20   let s = \"} if { while\";\n\
                \x20   let c = '{';\n\
                \x20   let q = '\\'';\n\
                \x20   let e = \"\\\"}\";\n\
                \x20   let r = r#\"{ \"if\" }\"#;\n\
                \x20   let b = b'}';\n\
                \x20   'outer: loop { break 'outer; }\n\
                \x20   if y {}\n\
                }\n\
                fn b() {}\n";
    assert_eq!(spans("a.rs", code), vec![span("a", 1, 2), span("b", 11, 1)]);
}

#[test]
fn multi_line_rust_strings_and_block_comments_hide_braces() {
    let code = "fn a() {\n    let s = \"\n{ if\n\";\n    /* { /* nested } */ if */\n}\n\
                fn b() {}\n";
    assert_eq!(spans("a.rs", code), vec![span("a", 1, 1), span("b", 7, 1)]);
}

#[test]
fn other_languages_hide_literals_and_comments() {
    let js =
        "function f() {\n  const u = \"http://x/{\";\n  const t = `${a} {`;\n  if (a) {} // }\n}\n";
    assert_eq!(spans("a.js", js), vec![span("f", 1, 2)]);
    let c = "int f(void) {\n#if X\n  return '}';\n#endif\n}\nint g(void) {\n}\n";
    assert_eq!(spans("a.c", c), vec![span("f", 1, 1), span("g", 6, 1)]);
}

#[test]
fn a_function_still_open_at_end_of_file_is_scored() {
    let code = "fn a() {\n    if x {\n        if y {}\n";
    assert_eq!(spans("a.rs", code), vec![span("a", 1, 3)]);
}

#[test]
fn wrapped_signature_with_extern_abi_parameter_is_scored() {
    let code = "pub fn register(\n    cb: extern \"C\" fn(i32),\n) {\n    if a {}\n}\n";
    assert_eq!(spans("a.rs", code), vec![span("register", 1, 2)]);
}

#[test]
fn named_function_inside_a_multi_line_call_callback_is_scored() {
    let code = "describe(\n  'suite',\n  () => {\n    function inner(x) {\n      if (x) {}\n    }\n  }\n);\n";
    assert_eq!(spans("a.js", code), vec![span("inner", 4, 2)]);
}

#[test]
fn brace_on_next_line_is_scored() {
    let code = "public class A\n{\n    public int Foo(int a)\n    {\n        if (a > 0) { return 1; }\n        return 0;\n    }\n}\n";
    assert_eq!(spans("A.cs", code), vec![span("Foo", 3, 2)]);
    let call = "configure(a)\nif (x)\n{\n}\n";
    assert!(spans("a.js", call).is_empty());
}

#[test]
fn attributes_and_raw_identifiers_are_scored() {
    let code = "#[inline] fn a() {\n    if x {}\n}\nfn r#type() {\n    if y {}\n}\n";
    assert_eq!(
        spans("a.rs", code),
        vec![span("a", 1, 2), span("r#type", 4, 2)]
    );
}

#[test]
fn python_async_def_is_scored() {
    let code = "async def f(x):\n    if x:\n        pass\n";
    assert_eq!(spans("a.py", code), vec![span("f", 1, 2)]);
}

#[test]
fn directive_lines_and_docstrings_are_not_code() {
    let swift = "func f() {\n#if DEBUG\n    if a {}\n#endif\n}\n";
    assert_eq!(spans("a.swift", swift), vec![span("f", 1, 2)]);
    // The brace scanner adds no span for a docstring holding `{`; the only
    // span is the indent scorer's (which still reads docstring words).
    let python =
        "def f(x):\n    \"\"\"Doc with { and\n    } words.\"\"\"\n    if x:\n        pass\n";
    assert_eq!(spans("a.py", python), vec![span("f", 1, 2)]);
}
