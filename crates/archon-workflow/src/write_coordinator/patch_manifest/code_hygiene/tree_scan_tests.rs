use super::*;

fn tree(path: &str, code: &str) -> Vec<(String, usize, u32)> {
    let scan = scan_functions(path, code);
    assert!(scan.unreliable.is_empty(), "{:?}", scan.unreliable);
    spans_of(scan.functions)
}

fn spans_of(functions: Vec<FunctionScore>) -> Vec<(String, usize, u32)> {
    functions
        .into_iter()
        .map(|function| (function.name, function.line, function.score))
        .collect()
}

fn span(name: &str, line: usize, score: u32) -> (String, usize, u32) {
    (name.to_string(), line, score)
}

#[test]
fn rust_wrapped_signatures_impl_methods_and_nested_items() {
    let code = "impl Store {\n\
                \x20   pub(crate) async fn load<T: Clone>(\n\
                \x20       &self,\n\
                \x20       key: &str,\n\
                \x20   ) -> Result<(), E> {\n\
                \x20       if let Some(x) = self.get(key) {}\n\
                \x20       let f = |a| if a && b { 1 } else { 2 };\n\
                \x20       fn inner() { if c {} }\n\
                \x20       println!(\"{:#?} if {{\", x);\n\
                \x20       Ok(())\n\
                \x20   }\n\
                \x20   #[inline] fn r#type(&self) {}\n\
                }\n\
                trait T {\n\
                \x20   fn decl(&self) -> u8;\n\
                \x20   fn with_default(&self) { for _ in 0..1 {} }\n\
                }\n";
    assert_eq!(
        tree("a.rs", code),
        vec![
            span("load", 2, 5),
            span("r#type", 12, 1),
            span("with_default", 16, 2)
        ]
    );
}

#[test]
fn go_functions_and_receiver_methods() {
    let code = "package p\n\
                func (s *Server) Handle(w Writer) {\n\tif x && y {}\n\tfor {}\n}\n\
                func Map[T any](xs []T) {\n\tf := func() { if z {} }\n\t_ = f\n}\n";
    assert_eq!(
        tree("a.go", code),
        vec![span("Handle", 2, 4), span("Map", 6, 2)]
    );
}

#[test]
fn tsx_arrow_functions_methods_and_jsx_apostrophes() {
    let code = "const Msg = ({ ok }: Props) => {\n\
                \x20 if (ok) { return <p>Couldn't {a && b}</p>; }\n\
                \x20 return null;\n\
                };\n\
                class A {\n\
                \x20 handle = async (e: Event) => { while (x) {} };\n\
                \x20 method(a: number): void { switch (a) { case 1: break; case 2: break; } }\n\
                }\n\
                function plain() { try { f(); } catch (e) { g(); } }\n\
                describe('suite', () => { it('works', () => { if (x) {} }); });\n";
    assert_eq!(
        tree("a.tsx", code),
        vec![
            span("Msg", 1, 3),
            span("handle", 6, 2),
            span("method", 7, 3),
            span("plain", 9, 2),
            span("describe('suite')", 10, 2)
        ]
    );
    let js = "export function f(a) {\n  return a.replace(/\"/g, '') || a;\n}\n";
    assert_eq!(tree("a.js", js), vec![span("f", 1, 2)]);
}

#[test]
fn python_async_def_docstrings_and_methods() {
    let code = "@decorator\n\
                async def fetch(x):\n\
                \x20   \"\"\"Return x if it is set, for example.\"\"\"\n\
                \x20   if x and y:\n\
                \x20       return [i for i in x if i]\n\
                \x20   elif z:\n\
                \x20       pass\n\
                \x20   def inner():\n\
                \x20       while True:\n\
                \x20           pass\n\
                class C:\n\
                \x20   def m(self):\n\
                \x20       try:\n\
                \x20           pass\n\
                \x20       except E:\n\
                \x20           pass\n";
    assert_eq!(
        tree("a.py", code),
        vec![span("fetch", 2, 6), span("m", 12, 2)]
    );
}

#[test]
fn java_methods_and_constructors() {
    let code = "class A {\n\
                \x20 A(int x) {\n    if (x > 0) {}\n  }\n\
                \x20 int f(int a) {\n    for (;;) {}\n    return a > 0 && b ? 1 : 2;\n  }\n\
                \x20 abstract void g();\n\
                }\n";
    assert_eq!(tree("A.java", code), vec![span("A", 2, 2), span("f", 5, 3)]);
}

#[test]
fn simple_functions_score_the_same_as_the_hand_scanner() {
    let cases = [
        (
            "a.rs",
            "fn f(a: u8) -> u8 {\n    if a > 1 && b {\n        for x in y {}\n    }\n    match a { _ => 0 }\n}\n",
        ),
        (
            "a.js",
            "function f(a) {\n  if (a || b) {\n    for (const x of y) {}\n  }\n  while (c) {}\n}\n",
        ),
        (
            "a.go",
            "func f(a int) int {\n\tif a > 1 && b {\n\t\tfor {}\n\t}\n\tswitch a {\n\tcase 1:\n\t}\n\treturn a\n}\n",
        ),
        (
            "A.java",
            "int f(int a) {\n  if (a > 1 || b) {\n    for (;;) {}\n  }\n  try { g(); } catch (E e) {}\n  return a;\n}\n",
        ),
        (
            "a.py",
            "def f(a):\n    if a:\n        for x in a:\n            pass\n    elif b:\n        pass\n",
        ),
    ];
    for (path, code) in cases {
        let hand: Vec<u32> = hand_scan(path, code)
            .functions
            .iter()
            .map(|f| f.score)
            .collect();
        let parsed: Vec<u32> = scan_functions(path, code)
            .functions
            .iter()
            .map(|f| f.score)
            .collect();
        assert_eq!(parsed, hand, "{path}");
    }
}

fn over_cap_body(branches: usize) -> String {
    (0..branches)
        .map(|idx| format!("    if c{idx} {{}}\n"))
        .collect()
}

#[test]
fn a_syntax_error_inside_a_function_is_noted_and_still_judged() {
    let code = format!("fn broken() {{\n    let x = ;\n{}}}\n", over_cap_body(20));
    let err = validate_complexity("src/a.rs", None, &code, 15).expect_err("judged");
    assert!(
        matches!(
            err,
            PatchError::FunctionWithSyntaxErrorTooComplex { line: 1, .. }
        ),
        "{err:?}"
    );
    let under = "fn broken() {\n    let x = ;\n}\n";
    let notes = validate_complexity("src/a.rs", None, under, 15).expect("under the cap");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].rule, "complexity_scan_unreliable");
    assert_eq!((notes[0].path.as_str(), notes[0].line), ("src/a.rs", 1));
    assert_eq!(notes[0].language, "rust");
    assert!(
        notes[0]
            .reason
            .contains("post-patch text: syntax error inside function 'broken'")
    );
}

#[test]
fn a_stray_syntax_error_is_noted_and_other_functions_still_judged() {
    let code = format!("fn heavy() {{\n{}}}\n}}}}\n", over_cap_body(20));
    let err = validate_complexity("src/a.rs", None, &code, 15).expect_err("still judged");
    assert!(
        matches!(err, PatchError::FunctionTooComplex { .. }),
        "{err:?}"
    );
    let clean = format!("fn heavy() {{\n{}}}\n", over_cap_body(20));
    let broken_baseline = format!("{clean}}}}}\n");
    let notes = validate_complexity("src/a.rs", Some(&broken_baseline), &clean, 15)
        .expect("a baseline that may hide a function is not judged against");
    assert!(notes.iter().any(|note| {
        note.reason
            .starts_with("baseline text: syntax error outside")
    }));
}

#[test]
fn a_hand_scanner_lost_sync_is_noted() {
    let code = "int ok(void) {\n}\nint broken(void) {\n  if (x) {\n";
    let notes = validate_complexity("src/a.c", None, code, 15).expect("not judged");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!((notes[0].line, notes[0].language.as_str()), (3, "c"));
    assert!(
        notes[0]
            .reason
            .contains("lost sync: function 'broken' never closed")
    );
}
