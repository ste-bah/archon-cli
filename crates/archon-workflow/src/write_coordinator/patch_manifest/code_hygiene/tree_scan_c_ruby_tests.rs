//! The C, C++ and Ruby grammars.

use super::*;

fn tree(path: &str, code: &str) -> Vec<(String, usize, u32)> {
    let scan = scan_functions(path, code);
    assert!(scan.parsed, "{path} was not parsed");
    assert!(scan.unreliable.is_empty(), "{:?}", scan.unreliable);
    scan.functions
        .into_iter()
        .map(|function| (function.name, function.line, function.score))
        .collect()
}

fn span(name: &str, line: usize, score: u32) -> (String, usize, u32) {
    (name.to_string(), line, score)
}

fn over_cap(indent: &str, statement: &str) -> String {
    (0..20)
        .map(|idx| format!("{indent}{}\n", statement.replace('N', &idx.to_string())))
        .collect()
}

#[test]
fn c_wrapped_signatures_directives_and_case_labels() {
    let code = "static int *\n\
                make_buffer(size_t size,\n\
                \x20           int flags) {\n\
                \x20   if (size > 0 && flags) { return 0; }\n\
                #if DEBUG\n\
                \x20   for (;;) {}\n\
                #endif\n\
                \x20   switch (flags) { case 1: break; case 2: break; }\n\
                \x20   do { } while (0);\n\
                \x20   return flags ? 0 : 1;\n\
                }\n";
    assert_eq!(tree("a.c", code), vec![span("make_buffer", 2, 7)]);
}

#[test]
fn cpp_out_of_line_methods_class_methods_templates_and_lambdas() {
    let code = "namespace n {\n\
                template <typename T>\n\
                T Widget::compute(\n\
                \x20   const T& a,\n\
                \x20   int b) const {\n\
                \x20 auto pick = [&](int v) { return v > 0 && a; };\n\
                \x20 try { if (b) {} } catch (...) {}\n\
                \x20 return a;\n\
                }\n\
                class Box {\n\
                \x20public:\n\
                \x20 int size() const { while (x) {} return 0; }\n\
                };\n\
                }\n\
                auto top = [](int x) { if (x) {} };\n";
    assert_eq!(
        tree("a.cpp", code),
        vec![
            span("Widget::compute", 3, 4),
            span("size", 12, 2),
            span("top", 15, 2)
        ]
    );
}

#[test]
fn a_header_is_read_as_c_unless_only_cpp_parses_it() {
    let c = "int area(int w, int h) {\n  if (w && h) { return w * h; }\n  return 0;\n}\n";
    assert_eq!(scan_functions("a.h", c).language, "c");
    let cpp = "class Box {\n public:\n  int size() const { if (x) {} return 0; }\n};\n";
    let scan = scan_functions("a.h", cpp);
    assert_eq!(scan.language, "cpp");
    assert_eq!(tree("a.h", cpp), vec![span("size", 3, 2)]);
}

#[test]
fn ruby_methods_singleton_methods_blocks_and_lambdas() {
    let code = "class Store\n\
                \x20 def load(key,\n\
                \x20          default = nil)\n\
                \x20   return default unless key\n\
                \x20   if key && default\n\
                \x20     1\n\
                \x20   elsif key\n\
                \x20     2\n\
                \x20   end\n\
                \x20   items.each do |item|\n\
                \x20     next if item.nil?\n\
                \x20   end\n\
                \x20 rescue StandardError\n\
                \x20   nil\n\
                \x20 end\n\
                \n\
                \x20 def self.build(x)\n\
                \x20   x or raise\n\
                \x20 end\n\
                end\n\
                describe \"thing\" do\n\
                \x20 it(\"works\") { expect(1).to eq(1) }\n\
                end\n\
                handler = ->(x) { x if x }\n";
    assert_eq!(
        tree("a.rb", code),
        vec![
            span("load", 2, 7),
            span("self.build", 17, 2),
            span("describe(\"thing\")", 21, 1),
            span("it(\"works\")", 22, 1),
            span("handler", 24, 2)
        ]
    );
}

#[test]
fn c_and_cpp_score_the_same_as_the_hand_scanner() {
    let cases = [
        (
            "a.c",
            "int f(int a) {\n  if (a > 1 && b) {\n    for (;;) {}\n  }\n  while (c) {}\n  return a;\n}\n",
        ),
        (
            "a.cpp",
            "int S::f(int a) {\n  if (a || b) {}\n  try { g(); } catch (E& e) {}\n  switch (a) { case 1: break; }\n  return a;\n}\n",
        ),
    ];
    for (path, code) in cases {
        let hand: Vec<u32> = hand_scan(path, code)
            .functions
            .iter()
            .map(|f| f.score)
            .collect();
        let parsed: Vec<u32> = tree(path, code)
            .into_iter()
            .map(|(_, _, score)| score)
            .collect();
        assert_eq!(parsed, hand, "{path}");
    }
}

#[test]
fn syntax_errors_in_c_cpp_and_ruby_functions_are_still_judged() {
    let c = format!(
        "int broken(void) {{\n  int x = ;\n{}}}\n",
        over_cap("  ", "if (cN) {}")
    );
    let cpp = format!(
        "void S::broken() {{\n  int x = ;\n{}}}\n",
        over_cap("  ", "if (cN) {}")
    );
    let rb = format!("def broken\n  x = (\n{}end\n", over_cap("  ", "y if cN"));
    for (path, code) in [("src/a.c", c), ("src/a.cpp", cpp), ("src/a.rb", rb)] {
        let err = validate_complexity(path, None, &code, 15).expect_err(path);
        assert!(
            matches!(err, PatchError::FunctionPartlyReadTooComplex { .. }),
            "{path}: {err:?}"
        );
    }
}
