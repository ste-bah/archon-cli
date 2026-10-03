use archon_workflow::task_universe::WorkflowV2DeliverableContract;
use archon_workflow::verifier_strength::{VerifierStrengthDefect, verifier_strength_defect};

fn contract(path: &str, min_instances: usize) -> WorkflowV2DeliverableContract {
    WorkflowV2DeliverableContract {
        kind: "artifact".into(),
        artifact_path: path.into(),
        min_instances,
        ..WorkflowV2DeliverableContract::default()
    }
}

#[test]
fn commands_that_cannot_fail_are_refused_but_predicates_are_allowed() {
    let cases = [
        ("test -f out.json", Some("out.json"), true),
        ("bash -c 'test -f out.json'", Some("out.json"), true),
        ("sh -lc \"test -f out.json\"", Some("out.json"), true),
        ("`test -f out.json`", Some("out.json"), true),
        ("test -f {artifact_path}", Some("out.json"), true),
        ("test -f {artifact_path};", Some("out.json"), true),
        ("test -f {artifact_path} && echo ok", Some("out.json"), true),
        (
            "test -f {artifact_path} && grep -q required {artifact_path}",
            Some("out.json"),
            false,
        ),
        ("test -s out.json", Some("out.json"), true),
        ("test -d out.json/", Some("out.json"), true),
        ("ls out.json", Some("out.json"), true),
        ("stat out.json", Some("out.json"), true),
        ("ls out.json.backup-*", Some("out.json.backup"), false),
        ("grep -q required out.json", Some("out.json"), false),
        (
            "bash -c 'grep -q required out.json'",
            Some("out.json"),
            false,
        ),
        ("true", None, true),
        (":", None, true),
        ("sh -c 'exit 0'", None, true),
        ("cargo --version", None, true),
        ("cargo -Vv", None, true),
        ("cargo --version && echo done", None, true),
        ("cargo --version && grep -q required out.json", None, false),
        ("python -c 'print(1)'", None, true),
        ("echo done", None, true),
        ("printf done", None, true),
        ("yes | head -1", None, true),
        ("find . | wc -l", None, true),
        ("probe || true", None, true),
        ("probe || :", None, true),
        ("jq -e '.ready == true' out.json; true", None, true),
        ("jq -e '.ready == true' out.json; :", None, true),
        ("jq -e '.ready == true' out.json; true;", None, true),
        ("jq -e '.ready == true' out.json; :;", None, true),
        ("jq -e '.ready == true' out.json; echo ok", None, true),
        ("jq -e '.ready == true' out.json; echo ok;", None, true),
        (
            "jq -e '.ready == true' out.json; grep -q required out.json",
            None,
            false,
        ),
        ("jq -e '.ready == true' out.json", None, false),
    ];

    for (command, own_artifact, refused) in cases {
        let defect = verifier_strength_defect(Some(command), own_artifact, None);
        assert_eq!(
            defect.is_some(),
            refused,
            "command: {command}; defect: {defect:?}"
        );
    }
}

#[test]
fn verifier_or_positive_instance_obligation_is_mandatory() {
    let no_floor = contract("out.json", 0);
    let positive_floor = contract("out.json", 1);

    let missing = verifier_strength_defect(None, Some("out.json"), Some(&no_floor))
        .expect("missing verifier without floor must be refused");
    assert!(
        missing
            .to_string()
            .contains("deleting the verifier does not satisfy")
    );
    assert!(
        verifier_strength_defect(None, Some("out.json"), Some(&positive_floor)).is_none(),
        "a positive floor is a falsifiable execution obligation"
    );
}

#[test]
fn own_artifact_existence_finding_demands_replacement_not_deletion() {
    let defect = verifier_strength_defect(Some("test -f out.json"), Some("out.json"), None)
        .expect("duplicate existence test must be refused")
        .to_string();
    assert!(defect.contains("replace"), "{defect}");
    assert!(
        defect.contains("deleting the verifier does not satisfy"),
        "{defect}"
    );
}

const AC_DL_002: &str = include_str!("fixtures/verifier_strength/ac_dl_002.sh");
const SUP_REQ_DL_013: &str = include_str!("fixtures/verifier_strength/sup_req_dl_013.sh");
const SUP_REQ_DL_132: &str = include_str!("fixtures/verifier_strength/sup_req_dl_132.sh");

fn defect(command: &str) -> Option<VerifierStrengthDefect> {
    verifier_strength_defect(Some(command), None, None)
}

/// Only the final top-level command decides a script's status; shapes whose
/// final command converts failure into success are still refused.
#[test]
fn final_fixed_success_commands_are_still_refused() {
    for command in [
        "probe || true",
        "probe || :",
        "probe; true",
        "probe\ntrue",
        "probe; exit 0",
        "probe\nexit 0",
        "echo ok",
        "python3 -c 'print(1)'",
        "prog --version",
        "bash -c 'probe || true'",
        "sh -c 'probe\n:'",
        "if probe; then exit 1; fi || true",
        "set -e; probe || true",
        "set -o pipefail\nprobe || true",
        "python3 - <<'PY'\nimport sys\nsys.exit(1)\nPY\necho done",
        "probe | head -1",
    ] {
        assert!(defect(command).is_some(), "must be refused: {command:?}");
    }
    assert!(matches!(
        defect("probe\ntrue"),
        Some(VerifierStrengthDefect::FixedSuccessFallback { .. })
    ));
    assert!(matches!(
        verifier_strength_defect(Some("test -f out.json\n"), Some("out.json"), None),
        Some(VerifierStrengthDefect::OwnArtifactExistenceOnly { .. })
    ));
}

/// R7 (wf-913e62ae) refused these real verifiers as fixed-success fallbacks:
/// every failure branch ends in `exit 1` and the heredoc program fails too.
#[test]
fn multi_line_verifiers_with_failing_branches_are_not_refused() {
    let without_errexit = |script: &str| script.replacen("set -eu\n", "", 1);
    for fixture in [AC_DL_002, SUP_REQ_DL_013, SUP_REQ_DL_132] {
        assert_eq!(defect(fixture), None, "{fixture}");
        assert_eq!(defect(&without_errexit(fixture)), None, "{fixture}");
    }
    for command in [
        "if bad; then echo x >&2; exit 1; fi",
        "if ! probe > out 2> err; then\n  echo 'probe failed' >&2\n  cat err >&2 || true\n  exit 1\nfi",
        "python3 - <<'PY'\nimport sys\nok = False  # it's not || true\nprint('a'); true\nsys.exit(0 if ok else 1)\nPY",
        "cat <<-EOF > x\n\tprobe || true; true\n\tEOF\ngrep -q ready x",
        "for f in a b; do grep -q x \"$f\"; done",
        "case \"$x\" in\n  ok) echo fine ;;\n  *) exit 1 ;;\nesac",
        "check() { grep -q x out || true; }\n{ check; grep -q y out; }",
        "while read -r line; do\n  test -n \"$line\" || exit 1\ndone < out",
        "test -f x || exit 1\necho verified",
        "set -eu\nprobe\necho verified",
        "set -o pipefail\nprobe | head -1",
        "test \"$(probe || true)\" = ready",
        "grep -q ready out; rc=$?; [ \"$rc\" -eq 0 ]",
        "echo 'unterminated",
    ] {
        assert_eq!(defect(command), None, "must not be refused: {command:?}");
    }
}

/// Only commands that run in the script's own shell can end it early: words
/// in quotes or arguments, subshells, uncalled function bodies and `kill` of
/// another process do not. Without errexit, an earlier guard never makes a
/// final `X || true` falsifiable, because X's own failure is still discarded.
#[test]
fn earlier_statements_end_the_script_only_through_live_commands() {
    for command in [
        "echo \"checking exit status\"\npytest || true",
        "echo 'kill switch'; pytest || true",
        "grep -q return log; pytest || true",
        "cleanup() { kill \"$PID\" || true; }\ntrap cleanup EXIT\npytest || true",
        "test -f out/a.json || exit 1\npytest || true",
        "if x; then exit 1; fi\ncmd || true",
        "f() { exit 1; }\npytest || true",
        "function f { exit 1; }\ntrue",
        "echo $(exit 1)\ntrue",
        "( exit 1 )\ntrue",
        "kill \"$PID\"\necho ok",
        "[ -f x ] || exit 0\necho ok",
        "pytest || exit 0\necho ok",
        "set -e\nset +e\npytest\ntrue",
        "echo pipefail; cargo test | tail -1",
        "pytest || true &",
        "pytest &",
        "pytest &\nwait",
        "cat <<EOF\r\nbody\r\nEOF\r\npytest || true",
        "pytest || /bin/true",
        "pytest || command true",
    ] {
        assert!(defect(command).is_some(), "must be refused: {command:?}");
    }
    for command in [
        "set -e\npytest\nrm -rf \"$T\" || true",
        "set -euo pipefail\npytest\necho done",
        "set -o errexit\npytest\ntrue",
        "pytest || { echo failed; exit 1; }\ntrue",
        "test -f out/a.json || exit 1\necho done",
        "grep -q x f || kill $$\necho ok",
        "grep -q x f || kill -TERM 0\necho ok",
        "check() { grep -q x out || exit 1; }\ncheck\necho ok",
        "trap 'exit 1' ERR\npytest\necho ok",
        "set -o pipefail\ncargo test | tail -n 5",
        "set -eo pipefail\ncargo test | tail -n 5",
    ] {
        assert_eq!(defect(command), None, "must not be refused: {command:?}");
    }
}
