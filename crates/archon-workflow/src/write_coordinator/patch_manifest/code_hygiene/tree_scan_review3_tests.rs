//! False rejections a third review confirmed with the locked grammars. Each
//! baseline -> post pair below leaves every over-cap function untouched, so
//! the ratchet must accept it.

use super::*;

const MAX: u32 = 15;

fn accepts(path: &str, baseline: &str, post: &str) {
    if let Err(err) = validate_complexity(path, Some(baseline), post, MAX) {
        panic!("{path}: untouched functions refused: {err}");
    }
}

fn names(path: &str, code: &str) -> Vec<(String, u32)> {
    scan_functions(path, code)
        .functions
        .into_iter()
        .map(|function| (function.name, function.score))
        .collect()
}

fn named(name: &str, score: u32) -> (String, u32) {
    (name.to_string(), score)
}

/// `if (a == N) return N;` lines, `count` of them.
fn c_ifs(indent: &str, count: usize) -> String {
    (0..count)
        .map(|n| format!("{indent}if (a == {n}) return {n};\n"))
        .collect()
}

fn c_big(indent: &str) -> String {
    format!(
        "{indent}int big(int a) {{\n{}{indent}  return 0;\n{indent}}}\n",
        c_ifs(&format!("{indent}  "), 16)
    )
}

#[test]
fn test_suite_callbacks_are_containers_not_one_function() {
    let cases: String = (0..16)
        .map(|n| {
            format!("  it('case {n}', () => {{\n    if (x{n}) {{ expect(1).toBe(1); }}\n  }});\n")
        })
        .collect();
    let helper = "  function helper(a) { return a && b; }\n";
    let baseline = format!("describe('Widget', () => {{\n{helper}{cases}}});\n");
    let added =
        "  it('new case', () => {\n    for (const y of ys) { expect(y).toBeTruthy(); }\n  });\n";
    let post = format!("describe('Widget', () => {{\n{helper}{cases}{added}}});\n");
    accepts("src/w.test.js", &baseline, &post);
    let read = names("src/w.test.js", &post);
    assert!(read.contains(&named("describe('Widget')", 1)), "{read:?}");
    assert!(read.contains(&named("helper", 2)), "{read:?}");
    assert!(read.contains(&named("it('case 0')", 2)), "{read:?}");
    assert!(read.contains(&named("it('new case')", 2)), "{read:?}");
}

#[test]
fn rspec_and_iife_wrappers_are_containers() {
    let its: String = (0..16)
        .map(|n| format!("  it 'does {n}' do\n    expect(x).to eq(1) if y{n}\n  end\n"))
        .collect();
    let baseline = format!("RSpec.describe Widget do\n  let(:x) {{ 1 }}\n{its}end\n");
    let post = format!(
        "RSpec.describe Widget do\n  let(:x) {{ 1 }}\n{its}  it 'new' do\n    expect(x).to eq(1) unless z\n  end\nend\n"
    );
    accepts("spec/w_spec.rb", &baseline, &post);
    let helpers: String = (0..8)
        .map(|n| format!("  function f{n}(a) {{ if (a) return 1; return a && b; }}\n"))
        .collect();
    let iife = |extra: &str| {
        format!("(function (root) {{\n{helpers}{extra}  root.lib = {{ f0 }};\n}})(this);\n")
    };
    accepts(
        "src/lib.js",
        &iife(""),
        &iife("  function f8(a) { if (a) return 1; return a && b; }\n"),
    );
    assert!(names("src/lib.js", &iife("")).contains(&named("f0", 3)));
}

#[test]
fn ruby_concern_blocks_scopes_and_endless_methods() {
    let code = "module Billable\n\
                \x20 extend ActiveSupport::Concern\n\
                \x20 class_methods do\n\
                \x20   def a(x)\n\
                \x20     return 1 if x && y\n\
                \x20     2\n\
                \x20   end\n\
                \x20 end\n\
                \x20 def plain(z)\n\
                \x20   z || 3\n\
                \x20 end\n\
                \x20 scope :active, -> { where(a: 1) }\n\
                \x20 scope :stale, -> { where(b: 2) }\n\
                end\n\
                class Foo\n\
                \x20 def self.build(x) = x ? 1 : 2\n\
                \x20 def empty; end\n\
                end\n";
    let read = names("app/models/billable.rb", code);
    for expected in [
        named("class_methods(...)", 1),
        named("a", 3),
        named("plain", 2),
        named("scope(:active)", 1),
        named("scope(:stale)", 1),
        named("self.build", 1),
    ] {
        assert!(read.contains(&expected), "{expected:?} not in {read:?}");
    }
    assert!(
        read.iter().all(|(name, _)| !name.starts_with('<')),
        "{read:?}"
    );
}

#[test]
fn a_baseline_the_grammar_misreads_still_pairs_untouched_functions() {
    let big_c = format!(
        "static int\nbig(int a)\n{{\n{}    return 0;\n}}\n",
        c_ifs("    ", 16)
    );
    let c_base = format!("#include <x.h>\nFOO_DECLARE(thing)\n\n{big_c}");
    accepts(
        "src/b.c",
        &c_base,
        &format!("{c_base}\nint small(void) {{ return 1; }}\n"),
    );
    let exported = format!(
        "#include <x.h>\nstatic int g(void)\n{{\n    return 0;\n}}\nEXPORT_SYMBOL(g)\n\n{big_c}"
    );
    accepts(
        "src/e.c",
        &exported,
        &format!("{exported}\nint small(void) {{ return 1; }}\n"),
    );
    let ts_big = format!(
        "export function big(a: number): number {{\n{}  return 0;\n}}\n",
        c_ifs("  ", 16).replace("==", "===")
    );
    let ts_base = format!("const x = <T,>(a: T) => a satisfies Foo;\n@@@\n{ts_big}");
    accepts(
        "src/b.ts",
        &ts_base,
        &format!("{ts_base}export const y = () => 1;\n"),
    );
    let handler = format!(
        "export const handler = async (req, res) => {{\n{}}};\n",
        c_ifs("  ", 16).replace("a ==", "req.a ===")
    );
    let route = format!(
        "app.get('/x', (req, res) => {{\n{}}});\n",
        c_ifs("  ", 16).replace("a ==", "req.a ===")
    );
    let js_base = format!("}}\n{handler}{route}");
    accepts("src/b2.js", &js_base, &format!("{js_base}const z = 1;\n"));
    let rust_ifs: String = (0..16)
        .map(|n| format!("        if a == {n} {{ return {n}; }}\n"))
        .collect();
    let rs_base = format!(
        "unsafe extern \"C\" {{\n    pub safe fn ext();\n}}\n\nstruct S;\nimpl S {{\n    pub fn big(&self, a: u32) -> u32 {{\n{rust_ifs}        0\n    }}\n}}\n\n\
         static T: std::sync::LazyLock<Vec<u32>> = std::sync::LazyLock::new(|| {{\n    let a = 3;\n{rust_ifs}    vec![]\n}});\n"
    );
    accepts(
        "src/b5.rs",
        &rs_base,
        &format!("{rs_base}\nfn small() {{}}\n"),
    );
}

#[test]
fn export_and_qt_macros_never_make_a_class_one_anonymous_function() {
    let class = |extra: &str| {
        format!(
            "namespace ns {{\nclass A {{\n{extra} public:\n{}}};\n}}\n",
            c_big("  ")
        )
    };
    accepts(
        "src/a.h",
        &format!("#pragma once\n{}", class("")),
        &format!("#pragma once\n{}", class("  Q_OBJECT\n")),
    );
    let trailing = format!(
        "namespace ns {{\nclass A {{\n public:\n{}  Q_OBJECT\n}};\n}}\n",
        c_big("  ")
    );
    accepts(
        "src/b4.cpp",
        &trailing,
        &format!("{trailing}int small() {{ return 1; }}\n"),
    );
    let template = format!(
        "template <typename T>\nstruct B {{\n{}}};\n",
        c_big("  ").replace("int big(int a)", "T big(T a)")
    );
    accepts("src/b7.h", &template, &format!("{template}MY_MACRO(x)\n"));
    let methods: String = (0..6)
        .map(|n| format!("  int m{n}(int a) {{ if (a) return 1; return a && b; }}\n"))
        .collect();
    let exported =
        format!("class API_EXPORT Foo : public Bar {{\n  Q_OBJECT\n public:\n{methods}}};\n");
    let read = names("src/n.cpp", &exported);
    assert!(
        read.iter().all(|(name, _)| !name.starts_with('<')),
        "{read:?}"
    );
    assert!(read.contains(&named("m0", 3)), "{read:?}");
    let k = "static int __init foo(void)\n{\n\tif (a) return 1;\n\treturn 0;\n}\n\
             STATIC_INLINE int bar(int x) { return x && y; }\n\
             int API baz(int x) { return x; }\n";
    let read = names("src/k.c", k);
    assert!(
        read.iter().all(|(name, _)| !name.starts_with('<')),
        "{read:?}"
    );
    assert!(read.iter().any(|(name, _)| name == "bar"), "{read:?}");
}

#[test]
fn a_misread_function_cannot_grow_unmeasured() {
    let foo = |count| {
        format!(
            "static int __init foo(int a)\n{{\n{}\treturn 0;\n}}\n",
            c_ifs("\t", count).replace("return 0;", "return 1;")
        )
    };
    let err = validate_complexity("src/i.c", Some(&foo(1)), &foo(30), MAX).expect_err("grew");
    let text = err.to_string();
    assert!(text.contains("'foo'"), "{text}");
    assert!(
        text.contains("could not read part of this function"),
        "{text}"
    );
    assert!(!text.contains("Fix the syntax error"), "{text}");
}

#[test]
fn callee_names_ignore_call_arguments() {
    let each = |rows: &str| {
        format!(
            "it.each([{rows}])('adds %i', (a, b) => {{\n  if (a) {{ expect(b).toBe(1); }}\n}});\n"
        )
    };
    let one = each("[1, 2], [3, 4]");
    let two = each("[1, 2], [3, 4], [5, 6]");
    assert_eq!(
        names("a.test.ts", &one),
        vec![named("it.each('adds %i')", 2)]
    );
    assert_eq!(names("a.test.ts", &one), names("a.test.ts", &two));
}

#[test]
fn ruby_case_counts_only_its_when_clauses() {
    let code = "def f(x)\n  case x\n  when 1 then :a\n  when 2 then :b\n  end\nend\n";
    assert_eq!(names("a.rb", code), vec![named("f", 3)]);
}

#[test]
fn more_source_extensions_are_checked() {
    for path in [
        "a.cjs", "a.mts", "a.cts", "a.inl", "a.ipp", "a.tpp", "a.rake",
    ] {
        assert!(checked_source(path), "{path}");
        assert!(scan_functions(path, "").parsed, "{path} has no grammar");
    }
}

#[test]
fn a_macro_the_hand_scanner_loses_sync_in_is_not_judged() {
    // A balanced token tree the hand scanner still misreads cannot be built
    // from valid Rust, so the rule is pinned on the macro-text reader.
    let (functions, notes) =
        tree_scan::macro_functions("{\n    fn inside() {\n        if a {}\n", 10);
    assert!(functions.is_empty(), "{functions:?}");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].0, 12);
    let (functions, _) = tree_scan::macro_functions("{\n    fn fine() { if a {} }\n}\n", 10);
    assert_eq!(
        (functions[0].name.as_str(), functions[0].line),
        ("fine", 12)
    );
}
