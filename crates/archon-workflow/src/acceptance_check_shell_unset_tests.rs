use super::*;

#[test]
fn finds_unset_read_and_names_its_line() {
    let reads =
        unset_reads("set -eu\ntest -f missing\ngrep -q x \"$UNSETVAR-x\" 2>/dev/null || true\n");
    assert_eq!(
        reads,
        vec![UnsetRead {
            name: "UNSETVAR".into(),
            line: 3
        }]
    );
    assert!(
        reads[0]
            .finding()
            .contains("set UNSETVAR or remove that line")
    );
    for set in [
        "set -u",
        "set -eu",
        "set -o nounset",
        "set -euo pipefail",
        "set -e -u",
    ] {
        assert_eq!(unset_reads(&format!("{set}\necho $UNSETVAR")).len(), 1);
    }
    assert_eq!(unset_reads("set -u; echo ${UNSETVAR}").len(), 1);
}

#[test]
fn leaves_bound_defaulted_literal_and_special_reads_alone() {
    for text in [
        "set -eu\nUNSETVAR=x\necho $UNSETVAR",
        "set -u\nexport UNSETVAR=x\necho $UNSETVAR",
        "set -u\nlocal UNSETVAR\necho $UNSETVAR",
        "set -u\nreadonly UNSETVAR=x\necho $UNSETVAR",
        "set -u\ndeclare UNSETVAR\necho $UNSETVAR",
        "set -eu\necho '${UNSETVAR}'",
        "set -eu\n# echo $UNSETVAR",
        "echo $UNSETVAR",
        "set -u\nfor v in a b; do echo $v; done",
        "set -u\nread -r v\necho $v",
        "set -u\necho $1 $@ $? $$",
        "set -u\necho $HOME $PATH $TMPDIR",
        "set -u\nx=$(echo ok)\necho $x",
        "set -u\nx=1\necho $((x+1))",
        "set -u\necho \\$UNSETVAR",
        "echo 'set -u'; echo $UNSETVAR",
        "set -u\necho 'UNSETVAR=x'", // quoted assignment isn't a read
        "set -u\nset +u\necho $UNSETVAR",
        "set -u\n. ./settings\necho $UNSETVAR",
        "set -u\nx=$((v=1))\necho $v",
        "set -u\nprintf -v v value\necho $v",
        "(\nset -u\n)\necho $UNSETVAR",
        "if false; then\nset -u\nfi\necho $UNSETVAR",
        "set -u\ncat <<'END'\n$UNSETVAR\nEND",
    ] {
        assert!(unset_reads(text).is_empty(), "{text}");
    }
    for form in [":-", "-", ":=", "=", ":+", "+"] {
        let text = format!("set -u\necho ${{UNSETVAR{form}}}\necho $UNSETVAR");
        assert!(unset_reads(&text).is_empty(), "{text}");
    }
}

#[test]
fn quoted_text_cannot_invent_bindings_or_options() {
    assert_eq!(
        unset_reads("set -u\necho 'UNSETVAR=x'\necho $UNSETVAR").len(),
        1
    );
    assert_eq!(
        unset_reads("set -u\necho \"UNSETVAR=x\"\necho $UNSETVAR").len(),
        1
    );
    assert!(unset_reads("echo \"set -u\"\necho $UNSETVAR").is_empty());
}

#[test]
fn runtime_requires_exact_line_name_exit_and_environment() {
    let text = "set -eu\necho $UNSETVAR";
    assert!(runtime_unset(
        text,
        1,
        "sh: line 2: UNSETVAR: unbound variable",
        &[]
    ));
    assert!(runtime_unset(
        text,
        2,
        "sh: 2: UNSETVAR: parameter not set",
        &[]
    ));
    for (code, stderr) in [
        (2, "sh: line 2: UNSETVAR: unbound variable"),
        (1, "sh: line 3: UNSETVAR: unbound variable"),
        (1, "script.sh: line 2: UNSETVAR: unbound variable"),
        (1, "sh: line 2: OTHER: unbound variable"),
    ] {
        assert!(!runtime_unset(text, code, stderr, &[]));
    }
    assert!(!runtime_unset(
        text,
        1,
        "sh: line 2: UNSETVAR: unbound variable",
        &["UNSETVAR"]
    ));
}
