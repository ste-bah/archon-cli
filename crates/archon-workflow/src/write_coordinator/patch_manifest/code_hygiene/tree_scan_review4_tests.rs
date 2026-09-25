//! Fail-open regressions a fourth review confirmed against the locked
//! grammars: logic hidden in nested callbacks, excuses that reached too far,
//! grammar choices that flipped on comments, and macro-mangled classes.

use super::*;

const MAX: u32 = 15;

fn accepts(path: &str, baseline: &str, post: &str) {
    if let Err(err) = validate_complexity(path, Some(baseline), post, MAX) {
        panic!("{path}: refused: {err}");
    }
}

fn refuses(path: &str, baseline: Option<&str>, post: &str, function: &str) {
    match validate_complexity(path, baseline, post, MAX) {
        Ok(notes) => panic!("{path}: accepted (notes {notes:?})"),
        Err(err) => assert!(
            err.to_string().contains(&format!("'{function}'")),
            "{path}: {err}"
        ),
    }
}

/// `count` copies of `statement` on one line.
fn repeat(statement: &str, count: usize) -> String {
    vec![statement; count].join(" ")
}

/// `count` lines of `if (NAME > N) { total++; }` at `indent`.
fn row_ifs(indent: &str, name: &str, count: usize) -> String {
    (1..=count)
        .map(|n| format!("{indent}if ({name}.v{n}) {{ total++; }}\n"))
        .collect()
}

#[test]
fn nested_callbacks_cannot_split_logic_under_the_cap() {
    let body = |wrapper: &str| {
        format!(
            "{wrapper} {{\n  const a = 1;\n  {}\n  [1].forEach(function (x) {{\n    {}\n    [2].forEach(function (y) {{\n      {}\n    }});\n  }});\n}}",
            repeat("if (a) {}", 13),
            repeat("if (x) {}", 13),
            repeat("if (y) {}", 13)
        )
    };
    refuses(
        "src/nest.js",
        None,
        &format!("({})();\n", body("function ()")),
        "forEach(...)",
    );
    let route = format!(
        "app.get('/', (req, res) => {{\n  {}\n  items.map((i) => {{\n    {}\n  }});\n}});\n",
        repeat("if (req.a) {}", 13),
        repeat("if (i) {}", 13)
    );
    refuses("src/routes.js", None, &route, "app.get('/')");
    let component = format!(
        "export default memo(forwardRef((props, ref) => {{\n  {}\n  return items.map((i) => {{\n    {}\n  }});\n}}));\n",
        repeat("if (props.a) {}", 13),
        repeat("if (i) {}", 13)
    );
    refuses("src/C.jsx", None, &component, "forwardRef(...)");
}

#[test]
fn an_error_in_one_test_excuses_nothing_elsewhere() {
    let opens = "  it('opens', () => {\n    const f = (x: ?string) => x;\n  });\n";
    let sums = "  it('sums rows', () => {\n    rows.forEach((row) => { total++; });\n  });\n";
    let baseline = format!("// @flow\ndescribe('orders', () => {{\n{opens}{sums}}});\n");
    let added = format!(
        "it('sums', () => {{\n  rows.forEach((row) => {{\n{}  }});\n}});\n",
        row_ifs("    ", "row", 16)
    );
    refuses(
        "src/o.test.js",
        Some(&baseline),
        &format!("{baseline}{added}"),
        "rows.forEach(...)",
    );
    let broken = "describe('a', () => {\n  it('x', () => { const y = 1 +; });\n});\n";
    let b = |extra: &str| {
        format!(
            "{broken}describe('b', () => {{\n  it('sums', () => {{ rows.forEach((row) => {{ total++; }}); }});\n{extra}}});\n"
        )
    };
    let fast = format!(
        "  it('sums fast', () => {{\n    rows.forEach((row) => {{\n{}    }});\n  }});\n",
        row_ifs("    ", "row", 16)
    );
    refuses(
        "src/b.test.js",
        Some(&b("")),
        &b(&fast),
        "rows.forEach(...)",
    );
    let spec = |extra: &str| {
        format!(
            "describe Order do\n  it \"matches\" do\n    case v in {{name: String => n}} then n end\n  end\n  it \"sums rows\" do\n    rows.each {{ |row| total += 1 }}\n  end\n{extra}end\n"
        )
    };
    let lines: String = (1..=16)
        .map(|n| format!("        total += 1 if row.v{n}\n"))
        .collect();
    let rb_fast =
        format!("  it \"sums rows fast\" do\n    rows.each do |row|\n{lines}    end\n  end\n");
    refuses(
        "spec/o_spec.rb",
        Some(&spec("")),
        &spec(&rb_fast),
        "rows.each(...)",
    );
}

#[test]
fn a_header_grammar_is_chosen_on_code_not_comments() {
    let grow = |comment: &str, count: usize| {
        let ifs: String = (1..=count)
            .map(|n| format!("    if (old > {n}) {{ new++; }}\n"))
            .collect();
        format!(
            "/* Helpers for this {comment} of buffers. */\nstatic inline int grow(int old, int new) {{\n{ifs}    return new;\n}}\n"
        )
    };
    accepts("src/c1.h", &grow("class", 20), &grow("kind", 20));
    accepts("src/c1.h", &grow("kind", 40), &grow("class", 40));
    refuses(
        "src/c1.h",
        Some(&grow("class", 20)),
        &grow("kind", 40),
        "grow",
    );
}

/// A class whose `big` holds `extra` lines then twenty ifs.
fn class(header: &str, before: &str, extra: &str, tail: &str) -> String {
    let ifs = "    if (a) { x++; }\n".repeat(20);
    format!(
        "{header} {{\n public:\n{before}  int big(int a) {{\n    int x = 0;\n{extra}{ifs}{tail}    return x;\n  }}\n}};\n"
    )
}

#[test]
fn macro_mangled_classes_are_read_method_by_method() {
    let ifdef =
        "#ifdef FAST\n    if (a > 1) {\n#else\n    if (a > 2) {\n#endif\n      x++;\n    }\n";
    refuses(
        "src/m.cpp",
        Some(&class("class API_EXPORT Foo", "", "", "")),
        &class("class Foo", "", ifdef, ""),
        "big",
    );
    refuses(
        "src/m.cpp",
        Some(&class("class API_EXPORT Foo", "", "", "")),
        &class("class API_EXPORT Foo", "", ifdef, ""),
        "big",
    );
    let raw = "    const char *quote = R\"(\")\";\n    x += quote[0];\n";
    accepts(
        "src/m.cpp",
        &class("class API_EXPORT Foo", "", "", raw),
        &class("class Foo", "", "", raw),
    );
    let never = "#if 0\n    { {\n#endif\n";
    accepts(
        "src/m.cpp",
        &class("class Foo", "", "", raw),
        &class("class API_EXPORT Foo", "", never, ""),
    );
    let method = |ifs: usize| {
        let body: String = (1..=ifs)
            .map(|n| format!("    if (v > {n}) {{ return {n}; }}\n"))
            .collect();
        format!("  int a(int v) {{\n{body}    return 0;\n  }}\n")
    };
    let aligned = |a: &str| {
        format!("class ALIGNAS(16) Foo {{\n public:\n{a}  int b(int v) {{ return v; }}\n}};\n")
    };
    refuses(
        "src/h.cpp",
        Some(&aligned(&method(3))),
        &aligned(&method(18)),
        "a",
    );
    let plain = |header: &str| format!("{header} {{\n public:\n{}}};\n", method(18));
    accepts(
        "src/h.cpp",
        &plain("class ALIGNAS(16) Foo"),
        &plain("class Foo"),
    );
    accepts(
        "src/h.cpp",
        &plain("class Foo"),
        &plain("class Q_DECL_EXPORT Foo"),
    );
}

#[test]
fn ruby_pattern_matching_counts_its_in_clauses() {
    let code = "def f(v)\n  case v\n  in Integer then 1\n  in String then 2\n  in [] then 3\n  end\nend\n\
                def g(v)\n  case v\n  when Integer then 1\n  when String then 2\n  end\nend\n";
    let read: Vec<(String, u32)> = scan_functions("a.rb", code)
        .functions
        .into_iter()
        .map(|function| (function.name, function.score))
        .collect();
    assert_eq!(read, vec![("f".to_string(), 4), ("g".to_string(), 3)]);
}

#[test]
fn conversion_operators_are_named_and_judged() {
    let ifs: String = (1..=20)
        .map(|n| format!("    if (v > {n}) {{ return true; }}\n"))
        .collect();
    let inline = format!(
        "struct Foo {{\n  int v;\n  explicit operator bool() const {{\n{ifs}    return false;\n  }}\n}};\n"
    );
    refuses("src/op.cpp", None, &inline, "operator bool");
    let outside = format!("Foo::operator int() const {{\n{ifs}  return 0;\n}}\n");
    refuses("src/op.cpp", None, &outside, "Foo::operator int");
}

#[test]
fn literal_receivers_and_chains_do_not_rename_a_callback() {
    let ifs: String = (1..=16)
        .map(|n| format!("    if (a > {n}) {{ t++; }}\n"))
        .collect();
    let table = |rows: &str| format!("[{rows}].forEach(([a, b]) => {{\n{ifs}}});\n");
    accepts(
        "src/t.test.js",
        &table("[1, 2], [3, 4]"),
        &table("[1, 2], [3, 4], [5, 6]"),
    );
    let chain = |links: &str| format!("fetch(u){links}.then((d) => {{\n{ifs}}});\n");
    accepts(
        "src/p.js",
        &chain(".then((r) => r.json())"),
        &chain(".then((r) => r.json()).then((j) => j.data)"),
    );
}

#[test]
fn a_hand_reading_that_lost_sync_never_joins_the_baseline() {
    let text =
        ")\nexport const h = async (req) => { if (a) {} };\nfunction broken() {\n  if (b) {\n";
    assert!(hand_scan("src/x.js", text).lost_sync);
    let names: Vec<String> = baseline_scan("src/x.js", text)
        .functions
        .into_iter()
        .map(|function| function.name)
        .collect();
    assert!(!names.iter().any(|name| name == "async"), "{names:?}");
}
