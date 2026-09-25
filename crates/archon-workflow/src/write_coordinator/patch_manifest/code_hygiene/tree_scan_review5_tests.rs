//! A fifth review's cases: containers recognised by structure rather than
//! by name, hand readings deduplicated by span, and macro-headed class
//! bodies read from the byte after their brace.

use super::*;

const MAX: u32 = 15;

fn accepts(path: &str, baseline: Option<&str>, post: &str) {
    if let Err(err) = validate_complexity(path, baseline, post, MAX) {
        panic!("{path}: refused: {err}");
    }
}

fn refuses(path: &str, post: &str, function: &str) {
    match validate_complexity(path, None, post, MAX) {
        Ok(notes) => panic!("{path}: accepted (notes {notes:?})"),
        Err(err) => assert!(
            err.to_string().contains(&format!("'{function}'")),
            "{path}: {err}"
        ),
    }
}

/// Ten branch points: three `if`s, `&&` `||`, a loop with an `if`, twice.
const TEN: &str = "if (a) { x(); } else if (b) { y(); } else if (c) { z(); }\n\
                   if (d && e || f) { w(); }\n\
                   for (;;) { if (g) break; }\n\
                   while (h) { if (i) j(); }\n";

/// Four branch points.
const THREE: &str = "if (a) { x(); } else if (b) { y(); }\nif (c && d) { z(); }\n";

/// `count` callbacks `open(...)` holding `body`, each closed by `close`.
fn held(open: &str, body: &str, close: &str, count: usize) -> String {
    (1..=count)
        .map(|n| format!("{}\n{body}{close}\n", open.replace('N', &n.to_string())))
        .collect()
}

#[test]
fn a_callback_holding_several_callbacks_is_a_container_whatever_its_name() {
    let go = format!(
        "package foo_test\nvar _ = Describe(\"Foo\", func() {{\n{}}})\n",
        held(
            "It(\"case N\", func() {",
            "if a { x() } else if b { y() }\nif c && d { z() }\n",
            "})",
            5
        )
    );
    accepts("foo_test.go", None, &go);
    let rake = format!(
        "namespace :db do\n{}end\n",
        held(
            "task :tN do",
            "if a then x elsif b then y end\nz if c && d\n",
            "end",
            5
        )
    );
    accepts("Rakefile.rb", None, &rake);
    let feature = format!(
        "RSpec.feature 'x' do\n{}end\n",
        held(
            "scenario 'cN' do",
            "if a then x elsif b then y end\nz if c && d\n",
            "end",
            5
        )
    );
    accepts("spec/f_spec.rb", None, &feature);
    let jquery = format!(
        "$(function () {{\n{}}});\n",
        held("$('#bN').on('click', function () {", THREE, "});", 5)
    );
    accepts("src/ui.js", None, &jquery);
    let node_test = format!(
        "import test from 'node:test';\ntest('top', async (t) => {{\n{}}});\n",
        held("await t.test('cN', () => {", THREE, "});", 5)
    );
    accepts("src/a.test.js", None, &node_test);
    for (open, case) in [
        (
            "test.describe('suite', () => {",
            "test('case N', async () => {",
        ),
        ("xdescribe('s', () => {", "xit('cN', () => {"),
        (
            "describe.skipIf(ci)('suite', () => {",
            "it('case N', async () => {",
        ),
    ] {
        let suite = format!("{open}\n{}}});\n", held(case, THREE, "});", 5));
        accepts("src/b.test.ts", None, &suite);
    }
    // A local helper called `it` changes nothing: structure decides.
    let routed = |call: &str| {
        format!(
            "const it = (_, f) => f();\napp.get('/', (req, res) => {{\n{}}});\n",
            held(&format!("{call}('', () => {{"), TEN, "});", 3)
        )
    };
    accepts("src/r.js", None, &routed("it"));
    accepts("src/r.js", None, &routed("run"));
}

#[test]
fn declarations_and_single_callback_chains_still_absorb() {
    let handler = format!(
        "function handler(req) {{\n{}}}\n",
        held("describe('', () => {", TEN, "});", 2)
    );
    refuses("src/h.js", &handler, "handler");
    let chained = |wrapper_open: &str, wrapper_close: &str| {
        format!(
            "{wrapper_open}\n  p.then(() => {{\n{TEN}    q.then(() => {{\n{TEN}    }});\n  }});\n{wrapper_close}\n"
        )
    };
    refuses(
        "src/c.js",
        &chained("(function(){", "}).call(this);"),
        "p.then(...)",
    );
    refuses(
        "src/d.js",
        &chained("((f) => f())(() => {", "});"),
        "p.then(...)",
    );
}

#[test]
fn a_parse_error_does_not_double_count_a_tree_function() {
    let suite = |cases: usize| {
        format!(
            "import defer * as m from \"m\";\ndescribe('s', () => {{\n{}}});\n",
            held("  it('cN', () => {", THREE, "  });", cases)
        )
    };
    accepts("src/d.test.ts", Some(&suite(4)), &suite(5));
    let methods = |count: usize| {
        let body: String = (1..=count)
            .map(|n| format!("  m{n}(a: number): void {{\n{THREE}  }}\n"))
            .collect();
        format!("import defer * as m from \"m\";\nexport class Svc {{\n{body}}}\n")
    };
    accepts("src/svc.ts", Some(&methods(3)), &methods(4));
    let g = "function g(a) {\n  switch (a) { case 1: x(); break; case 2: y(); break; }\n  items.forEach(function (i) { if (i) { k(); } });\n";
    let g = format!("{g}{TEN}  if (m) {{ n(); }} else if (o) {{ p(); }}\n  return a;\n}}\n");
    accepts("src/g.js", Some(&g), &format!("const y = a |> f(%);\n{g}"));
    let catch = |sections: usize| {
        format!(
            "#include \"catch.hpp\"\nDECLARE_FIXTURE(Foo)\nTEST_CASE(\"suite\") {{\n{}}}\n",
            held("  SECTION(\"sN\") {", THREE, "  }", sections)
        )
    };
    // A declared C++ function absorbs its macro blocks (Catch2 `SECTION`s):
    // counted once, it genuinely grows 13 -> 17.
    let err = validate_complexity("src/t.cpp", Some(&catch(3)), &catch(4), MAX).expect_err("grew");
    assert!(err.to_string().contains("was 13, now 17"), "{err}");
}

/// `big` scores 17 in each layout.
const BIG: &str = "    if (a) { x(); } else if (b) { y(); } else if (c) { z(); }\n\
                   \x20   if (d && e || f) { w(); }\n\
                   \x20   for (;;) { if (g) break; }\n\
                   \x20   while (h) { if (i && j || k) l(); }\n\
                   \x20   if (m) { n(); } else if (o) { p(); }\n\
                   \x20   if (q) r();\n\
                   \x20   if (s) t();\n\
                   \x20   return 0;\n";

#[test]
fn a_macro_headed_class_body_is_read_from_the_byte_after_its_brace() {
    let layouts = [
        format!("class API_EXPORT Foo {{\npublic:\n  int big(int a) {{\n{BIG}  }}\n}};\n"),
        format!(
            "class API_EXPORT Foo {{ /*\n  f( {{ */\npublic:\n  int big(int a) {{\n{BIG}  }}\n}};\n"
        ),
        format!("class API_EXPORT Foo {{ public: int big(int a) {{\n{BIG}  }}\n}};\n"),
        format!("class API_EXPORT Foo {{\npublic:\n  int big(int a) {{\n{BIG}  }} }};\n"),
    ];
    for layout in layouts {
        refuses("src/cls.cpp", &layout, "big");
    }
}

#[test]
fn dead_preprocessor_branches_with_comments_or_parentheses_are_skipped() {
    for dead in ["#if 0 /* note */", "#if (0)", "#if 0 // off"] {
        let code = format!("int f(void) {{\n{dead}\n  {{ {{\n#endif\n  if (a) {{}}\n}}\n");
        let scan = hand_scan("a.cs", &code);
        assert!(!scan.lost_sync, "{dead}");
        assert_eq!(scan.functions[0].score, 2, "{dead}");
    }
}

#[test]
fn identical_blocks_do_not_share_an_error_excuse() {
    let rows: String = (1..=16)
        .map(|n| format!("      if (row.v{n}) {{ t++; }}\n"))
        .collect();
    let suite = |second: &str| {
        format!(
            "describe('s', () => {{\n  beforeEach(() => {{ const y = 1 +; }});\n  beforeEach(() => {{\n{second}  }});\n  it('a', () => {{}});\n}});\n"
        )
    };
    let added = format!("    rows.forEach((row) => {{\n{rows}    }});\n");
    match validate_complexity("src/e.test.js", Some(&suite("")), &suite(&added), MAX) {
        Ok(notes) => panic!("excused: {notes:?}"),
        Err(err) => assert!(err.to_string().contains("rows.forEach"), "{err}"),
    }
}
