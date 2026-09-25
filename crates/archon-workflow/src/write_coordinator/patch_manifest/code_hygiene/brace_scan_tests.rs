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
fn wrapped_signature_is_named_from_its_header_line() {
    let code = "use a::b;\n\
                \n\
                fn validate_input(\n\
                \x20   history: &X,\n\
                \x20   publication: &Y,\n\
                ) -> Result<(), StoreError> {\n\
                \x20   if a && b {\n\
                \x20   }\n\
                \x20   Ok(())\n\
                }\n\
                fn next() {\n\
                \x20   if c {}\n\
                }\n";
    assert_eq!(
        spans("src/a.rs", code),
        vec![span("validate_input", 3, 3), span("next", 11, 2)]
    );
}

#[test]
fn wrapped_signature_without_paren_on_brace_line_is_not_started_mid_body() {
    let code = "pub fn load(\n\
                \x20   key: &str,\n\
                ) -> Result<Row, Error> {\n\
                \x20   if let Some(x) = cache.get(key) {\n\
                \x20       return Ok(x);\n\
                \x20   }\n\
                \x20   match fetch(key) {\n\
                \x20       Ok(row) => Ok(row),\n\
                \x20       Err(e) => Err(e),\n\
                \x20   }\n\
                }\n";
    assert_eq!(spans("src/a.rs", code), vec![span("load", 1, 3)]);
}

#[test]
fn control_flow_outside_a_function_never_starts_one() {
    let rust = "}\nif let Some(x) = y {\n    if a {}\n}\nwhile let Some(v) = it.next() {\n}\n";
    assert!(spans("src/a.rs", rust).is_empty());
    let swift = "if let x = f(y) {\n}\nguard let v = g(w) else {\n}\n";
    assert!(spans("src/a.swift", swift).is_empty());
    let closures = "thread::spawn(move || {\n    if a {}\n});\n";
    assert!(spans("src/a.rs", closures).is_empty());
}

#[test]
fn a_header_inside_a_top_level_string_is_dropped_at_the_string_end() {
    let code = "const SRC: &str = \"\n\
                fn quoted(\n\
                \";\n\
                impl Store {\n\
                \x20   fn real(&self) {\n\
                \x20       if x {}\n\
                \x20   }\n\
                }\n";
    assert_eq!(spans("src/a.rs", code), vec![span("real", 5, 2)]);
}

#[test]
fn bodiless_declarations_are_dropped_at_their_semicolon() {
    let code = "trait Store {\n\
                \x20   fn get(&self, key: [u8; 32]) -> u8;\n\
                \x20   fn put(\n\
                \x20       &self,\n\
                \x20       key: [u8; 32],\n\
                \x20   ) -> Result<(), Error>;\n\
                }\n\
                fn real() {\n\
                \x20   if x {}\n\
                }\n";
    assert_eq!(spans("src/a.rs", code), vec![span("real", 8, 2)]);
}

#[test]
fn qualified_and_generic_headers_are_named_without_generics() {
    let code = "pub(crate) async fn fetch_all<T: Clone + Send>(\n\
                \x20   items: &[T; 4],\n\
                ) -> Vec<T>\n\
                where\n\
                \x20   T: Default,\n\
                {\n\
                \x20   for i in items {}\n\
                }\n\
                pub fn g<T>(x: T) -> T { x }\n\
                const unsafe fn h() {}\n\
                pub(in crate::a) extern \"C\" fn k() {}\n";
    assert_eq!(
        spans("src/a.rs", code),
        vec![
            span("fetch_all", 1, 2),
            span("g", 9, 1),
            span("h", 10, 1),
            span("k", 11, 1),
        ]
    );
}

#[test]
fn a_function_is_scored_from_its_header_to_its_closing_brace() {
    let mut code = String::from("fn heavy(\n    a: u8,\n) -> u8 {\n");
    for idx in 0..16 {
        code.push_str(&format!("    if c{idx} {{}}\n"));
    }
    code.push_str("    0\n}\nfn light() {}\n");
    assert_eq!(
        spans("src/a.rs", &code),
        vec![span("heavy", 1, 17), span("light", 22, 1)]
    );
}

#[test]
fn other_brace_languages_track_wrapped_parameter_lists() {
    let java = "public static int compute(\n    int a,\n    int b) {\n    if (a > b) { return a; }\n    return b;\n}\n";
    assert_eq!(spans("src/A.java", java), vec![span("compute", 1, 2)]);
    let call = "configure(\n    a,\n    b\n)\nif (x) {\n}\n";
    assert!(spans("src/a.js", call).is_empty());
    let legacy = "describe('x', () => {\n  if (a) {}\n});\n";
    assert_eq!(spans("src/a.js", legacy), vec![span("describe", 1, 2)]);
    let one_line = "int main(void) {\n  return 0;\n}\n";
    assert_eq!(spans("src/a.c", one_line), vec![span("main", 1, 1)]);
}

#[test]
fn python_is_scored_only_by_the_indent_scorer() {
    let code = "def f(x):\n    d = {\"a\": 1}\n    if x:\n        return d\n";
    assert_eq!(spans("src/a.py", code), vec![span("f", 1, 2)]);
    // A wrapped `def` adds no brace-scanner span: its `(` closes on `):`
    // with no `{`. (The indent scorer's own reading of it is unchanged.)
    let wrapped = "def f(\n    a,\n    b,\n):\n    if a:\n        return b\n";
    assert_eq!(spans("src/a.py", wrapped), python_spans(wrapped));
}

fn python_spans(code: &str) -> Vec<(String, usize, u32)> {
    python_scores(code)
        .into_iter()
        .map(|function| (function.name, function.line, function.score))
        .collect()
}

#[test]
fn rejection_names_function_and_line() {
    let mut code = String::from("//! doc\npub fn wide(\n    a: u8,\n) -> Result<(), E> {\n");
    for idx in 0..16 {
        code.push_str(&format!("    if c{idx} {{}}\n"));
    }
    code.push_str("    Ok(())\n}\n");
    let err = validate_complexity("src/a.rs", &code, 15).expect_err("too complex");
    let text = err.to_string();
    assert!(
        text.contains("function 'wide' at line 2 of 'src/a.rs'"),
        "{text}"
    );
    assert!(text.contains("complexity 17, exceeds max 15"), "{text}");
    assert!(text.contains("smaller helper functions"), "{text}");
}
