//! Issue-599: exact template refusals and accepted concrete shell commands.

use super::tests::run_verifier;
use super::{ContractRoots, typed_verification_command};

#[test]
fn shell_template_domain_table() {
    let roots = ContractRoots::project_only("/project".to_string());
    // Every outcome is exact, including the validator-owned refusal text.
    let rows = [
        ("${X}", true),
        (" ${X} ", true),
        ("$X", true),
        ("\"$X\"", true),
        ("\"${X}\"", true),
        ("${1}", true),
        ("${X:-}", true),
        ("${X}${Y}", true),
        ("$X && $Y", true),
        ("$X; $Y", true),
        ("$X | $Y", true),
        ("VAR=1 $CMD", true),
        ("VAR=1", true),
        ("${X:+d}", true),
        ("${X+d}", true),
        ("`$X`", true),
        ("$($X)", true),
        ("<cmd>", true),
        ("$X $Y", true),
        ("$1", true),
        ("$@", true),
        ("$*", true),
        ("$!", true),
        ("${@}", true),
        ("${1:-}", true),
        ("A+=1 $X", false),
        ("$(true)", false),
        ("a=$(echo x) $X", false),
        ("${X:-$(true)}", false),
        ("R=$(false)", false),
        ("OUT=$(make -q nosuch)", false),
        ("R=$($X)", true),
        ("${X:- }", true),
        (">out", true),
        ("$X <<EOF\nhello\nEOF", true),
        ("cat <<EOF\n$X\nEOF", false),
        ("$X 2>/dev/null", true),
        ("$X >/dev/null", true),
        ("$X 2>&1", true),
        ("$X # c", true),
        ("{ $X; }", true),
        ("( $X )", true),
        ("${X:=}", true),
        ("${X%x}", true),
        ("${X/a/b}", true),
        ("${X:0:1}", true),
        ("\"$X\"\"$Y\"", true),
        ("${X:-$Y}", true),
        ("! $X", false),
        ("! ${X}", false),
        ("$X && echo ok", false),
        ("$X || true", false),
        ("a=$(x) b", false),
        ("'${X}'", false),
        ("${X:-${Y:-z}}", false),
        ("$((1+1))", false),
        ("${X?msg}", false),
        ("(cd $X)", false),
        ("${X:-d}", false),
        ("echo ${X}", false),
        ("$HOME/bin/run", false),
        ("$PYTHON -m pytest", false),
        ("${CARGO:-cargo} test", false),
        ("\"$BIN\" --check", false),
        ("$CMD --flag", false),
        ("VAR=1 $CMD --flag", false),
        ("$(which jq) .", false),
        ("${X}/run.sh", false),
        ("test -f \"$F\"", false),
        // Recursive defaults, remaining list separators, quotes and assignments.
        ("${X-d}", false),
        ("${X-}", true),
        ("${X:-${Y:-d}}", false),
        ("${X:-${Y}}", true),
        ("${X:-\"\"}", true),
        ("$X || $Y", true),
        ("$X & $Y", true),
        ("$X\n$Y", true),
        ("VAR=1 OTHER=2 $X $Y", true),
        ("$X; echo ok", false),
        ("$(echo '$X; $Y')", false),
        ("'$X'", false),
        ("${#X}", false),
        ("${X%pattern}", true),
        ("${X:=d}", false),
        ("${X?error}", false),
        ("<optional: x>", true),
        ("  <optional: x>  ", true),
        ("  ${NAME_1}  ", true),
        ("${X} \"$Y\"", true),
        ("python3 - <<'PY'\nassert 1 == 1\nPY", false),
        ("cmd < in 2>/dev/null", false),
        ("test -s \"$HOME/x\"", false),
        ("test -s \"${VAR:-d}\"", false),
        ("test \"${N}\" -gt 0", false),
        ("test \"$(cmd)\" = expected", false),
        ("test \"$VAR\" = expected", false),
    ];
    for (command, refused) in rows {
        let contract = serde_json::json!({
            "artifact_path": "out.json", "typed_verifier_command": command,
        });
        let defects = super::contract_defects(&contract);
        let expected: Vec<_> = if refused {
            vec![crate::defect::ValidationDefect::new(
                "unexpanded_contract_field",
                "contract",
                "typed_verifier_command",
                "deliverable contract typed_verifier_command has no literal program word once assignments, redirections and placeholder expansions are removed, so it cannot exercise the deliverable; replace it with the real command",
            )]
        } else {
            vec![]
        };
        assert_eq!(defects, expected, "{command}");
        assert_eq!(
            typed_verification_command(&roots, &contract).as_deref(),
            (!refused).then_some(command.trim()),
            "{command}"
        );
        if refused {
            let (passed, output) = run_verifier(&super::super::deliverable_native_tests::command(
                &roots, &contract,
            ));
            assert!(!passed, "{command}: {output}");
            assert!(output.contains(&expected[0].message), "{command}: {output}");
        }
    }
}
