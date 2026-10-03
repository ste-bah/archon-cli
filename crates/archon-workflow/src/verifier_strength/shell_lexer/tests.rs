use super::{ListOp, Statement, statements};

const AC_DL_002: &str = include_str!("../../../tests/fixtures/verifier_strength/ac_dl_002.sh");
const SUP_REQ_DL_013: &str =
    include_str!("../../../tests/fixtures/verifier_strength/sup_req_dl_013.sh");
const SUP_REQ_DL_132: &str =
    include_str!("../../../tests/fixtures/verifier_strength/sup_req_dl_132.sh");

fn parsed(script: &str) -> Vec<Statement> {
    statements(script).unwrap_or_else(|| panic!("structure must be certain: {script}"))
}

fn last_text(script: &str) -> String {
    parsed(script).last().expect("a statement").text.clone()
}

fn last_compound(script: &str) -> bool {
    parsed(script)
        .last()
        .expect("a statement")
        .items
        .iter()
        .any(|item| item.stages.iter().any(|stage| stage.compound))
}

#[test]
fn newline_semicolon_and_ampersand_separate_statements() {
    let script = "a; b\nc & d";
    let parsed = parsed(script);
    let texts: Vec<_> = parsed.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(texts, ["a", "b", "c", "d"]);
    assert!(parsed[2].background && !parsed[3].background);
    assert_eq!(
        last_text("a 2>&1 >&2 &>/dev/null"),
        "a 2>&1 >&2 &>/dev/null"
    );
}

#[test]
fn and_or_lists_and_pipelines_are_split_at_top_level_only() {
    let parsed = parsed("a | b && c ||\n  d");
    let [statement] = parsed.as_slice() else {
        panic!("{parsed:?}")
    };
    let ops: Vec<_> = statement.items.iter().map(|item| item.op).collect();
    assert_eq!(ops, [None, Some(ListOp::And), Some(ListOp::Or)]);
    assert_eq!(statement.items[0].stages.len(), 2);
    let nested = statements("test \"$(a || true)\" = x && echo $(b | c; d)").unwrap();
    assert_eq!(nested.len(), 1);
    assert_eq!(nested[0].items.len(), 2);
    assert_eq!(nested[0].items[1].stages.len(), 1);
}

#[test]
fn heredoc_bodies_are_skipped_for_every_delimiter_form() {
    for open in ["<<PY", "<<'PY'", "<<\"PY\"", "<< PY", "<<\\PY"] {
        let script = format!(
            "python3 - {open}\nprint(\"don't\") || true; true\nx = '\nPY\npython3 - <<-'EOF'\n\tit's; true\n\tEOF"
        );
        let parsed = parsed(&script);
        assert_eq!(parsed.len(), 2, "{open}: {parsed:?}");
        assert_eq!(parsed[0].text, "python3 - <<PY");
        assert_eq!(parsed[1].text, "python3 - <<EOF");
    }
    assert!(statements("cat <<EOF\nnever terminated").is_none());
    assert!(statements("cat <<EOF").is_none());
    assert_eq!(last_text("cat <<<'here; true'"), "cat <<<'here; true'");
}

#[test]
fn comments_are_dropped_even_with_unbalanced_quotes() {
    assert_eq!(last_text("a # it's || true\n# don't; true"), "a");
    assert_eq!(last_text("echo a#b"), "echo a#b");
    assert_eq!(last_text("echo ${#x} $#"), "echo ${#x} $#");
}

#[test]
fn compound_commands_are_one_nested_unit() {
    for script in [
        "if bad; then echo x >&2; exit 1; fi",
        "if ! probe; then\n  cat err >&2 || true\n  exit 1\nelif x; then :; else y; fi",
        "while read l; do echo \"$l\"; done < <(ls)",
        "until a; do b; done",
        "for f in a b; do test -f \"$f\" || exit 1; done",
        "for ((i=0; i<3; i++)); do :; done",
        "case \"$x\" in\n  a|b) echo ok ;;\n  (c) if y; then z; fi ;;\n  *) exit 1\nesac",
        "{ a; b || true; }",
        "( cd x && b )",
        "f() { a || true; }",
        "function g { exit 1; }",
        "[[ -f x && $y == z || -d w ]]",
        "(( n > 0 ))",
    ] {
        let parsed = parsed(script);
        assert_eq!(parsed.len(), 1, "{script}: {parsed:?}");
        assert!(last_compound(script), "{script}");
    }
    assert!(!last_compound(
        "x=$(if a; then b; fi) && arr=(1 2) && echo \"$x\""
    ));
}

#[test]
fn uncertain_structure_is_none() {
    for script in [
        "echo 'unterminated",
        "echo \"unterminated",
        "if a; then b",
        "while a; do b",
        "a; fi",
        "a; done",
        "a )",
        "{ a",
        "a &&",
        "a |",
        "&& a",
        "a;; b",
        "echo $(a",
        "echo `a",
        "  <redacted>  if a; then b; fi",
        "trailing \\",
    ] {
        assert!(statements(script).is_none(), "{script}");
    }
}

#[test]
fn real_verifiers_end_in_their_final_heredoc_or_command() {
    assert_eq!(last_text(AC_DL_002), "python3 - \"$tmp\" <<PY");
    assert_eq!(last_text(SUP_REQ_DL_013), "python3 - \"$T\" <<PYVERIFY");
    let dl_013 = parsed(SUP_REQ_DL_013);
    assert!(
        dl_013.iter().any(
            |statement| statement.background && statement.text == "python3 - \"$T\" <<PYSERVER"
        )
    );
    assert!(last_text(SUP_REQ_DL_132).starts_with("echo 'SUP-REQ-DL-132 verified:"));
    for fixture in [AC_DL_002, SUP_REQ_DL_013, SUP_REQ_DL_132] {
        assert!(!last_compound(fixture));
        assert!(
            parsed(fixture)
                .iter()
                .all(|statement| !statement.text.contains("import ")),
            "heredoc bodies must not reach statement text"
        );
    }
}
