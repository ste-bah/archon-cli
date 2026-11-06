use super::*;

#[test]
fn finds_unset_read_and_names_its_line() {
    let reads =
        unset_reads("set -eu\ntest -f missing\ngrep -q x \"$UNSETVAR-x\" 2>/dev/null || true\n");
    assert_eq!(
        reads,
        vec![UnsetRead {
            name: "UNSETVAR".into(),
            line: 3,
            lines: vec![3]
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
fn runtime_uses_whole_check_and_run_bindings() {
    let text = "set -eu\necho $UNSETVAR";
    for diagnostic in [
        "sh: line 99: UNSETVAR: unbound variable",
        "sh: 2: UNSETVAR: parameter not set",
    ] {
        assert!(runtime_unset(text, 1, diagnostic, &[]));
        assert!(!runtime_unset(text, 0, diagnostic, &["UNSETVAR"]));
        assert!(!runtime_unset(
            "set -u; UNSETVAR=x; echo $UNSETVAR",
            1,
            diagnostic,
            &[]
        ));
    }
    assert!(!runtime_unset(
        text,
        1,
        "script.sh: line 2: UNSETVAR: unbound variable",
        &[]
    ));
    assert!(!runtime_unset(
        text,
        1,
        "sh: line 2: OTHER: unbound variable",
        &[]
    ));
}

#[test]
fn analyses_real_check_regions_and_deduplicates_names() {
    for preamble in [
        "d=$(mktemp -d)\ntrap 'rm -rf \"$d\"' EXIT",
        "find . -name x",
        "result=$(printf '%s' \"value\")",
        "cat <<'END'\n$LITERAL\nEND",
        "echo \\é",
    ] {
        let reads = unset_reads(&format!(
            "set -eu\n{preamble}\necho \"$MISSING\"\necho $MISSING"
        ));
        assert_eq!(reads.len(), 1, "{preamble}");
        assert_eq!(reads[0].name, "MISSING");
        assert_eq!(reads[0].lines.len(), 2);
    }
    assert!(unset_reads("set -u\ncat <<'END'\n$MISSING\nEND").is_empty());
    assert_eq!(unset_reads("set -u\ncat <<END\n'$MISSING'\nEND")[0].line, 3);
}

#[test]
fn accepts_shell_bindings_guards_and_policy_names() {
    for code in [
        "X+=a\necho $X",
        "(( X = 1 ))\necho $X",
        "read\necho $REPLY",
        "[[ -v X ]] && echo \"$X\"",
        "echo $FUNCNAME $GROUPS $SHELLOPTS $BASH_SUBSHELL $BASHPID $EPOCHSECONDS $BASH_SOURCE",
        "echo $ARCHON_WORKFLOW_RESOURCE_CLASS $CARGO_BUILD_JOBS $CARGO_INCREMENTAL $GOCACHE $npm_config_cache $YARN_CACHE_FOLDER $PIP_CACHE_DIR $UV_CACHE_DIR",
    ] {
        assert!(unset_reads(&format!("set -u\n{code}")).is_empty(), "{code}");
    }
    let mut policy = crate::acceptance_check_environment::CheckPolicy::default();
    policy.bound.insert("CHECK_MODE".into(), "test".into());
    policy.forwarded.push("INPUT_DATA".into());
    let bound = crate::acceptance_check_environment::policy_bound_names(&policy);
    assert!(unset_reads_with_bound("set -u\necho $CHECK_MODE $INPUT_DATA", &bound).is_empty());
    // The host commonly sets this, but it is not in the check environment.
    assert_eq!(unset_reads("set -u\necho $CODEX_HOME").len(), 1);
}

#[test]
fn literal_heredocs_and_shell_arrays_are_not_unset_reads() {
    for delimiter in ["\\EOF", "'EOF'", "\"EOF\""] {
        assert!(unset_reads(&format!("set -u\ncat <<{delimiter}\n$MISSING\nEOF\n")).is_empty());
    }
    assert_eq!(
        unset_reads("set -u\ncat <<-EOF\n\t$MISSING\n\tEOF\n")[0].name,
        "MISSING"
    );
    assert!(unset_reads("set -u\ncat <<-'EOF'\n\t$MISSING\n\tEOF\n").is_empty());
    assert!(unset_reads("set -u; echo $PIPESTATUS $DIRSTACK $COPROC $BASH_VERSINFO").is_empty());
}

#[test]
fn every_draft_prefix_is_panic_free() {
    for source in [
        "set -u; echo `test \\",
        "set -u; echo `echo \\`x`",
        "set -u; echo \"$(echo '${X:-value}')\"",
        "set -u; echo ${X:?message}",
        "set -u\ncat <<\\EOF\n$X\nEOF\n",
        "set -u; echo 'text\\",
        "set -u; echo $(echo \\",
        "set -u; echo `é\\",
    ] {
        for end in 0..=source.len() {
            if source.is_char_boundary(end) {
                let draft = &source[..end];
                assert!(
                    std::panic::catch_unwind(|| unset_reads(draft)).is_ok(),
                    "{draft:?}"
                );
            }
        }
    }
}
