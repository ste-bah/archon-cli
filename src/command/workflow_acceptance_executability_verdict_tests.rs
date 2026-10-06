//! Issue 328: which failed runs gave a verdict, decided from the check's
//! own text and its search path, on captured output shapes.

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::AcceptanceContract;

use super::{Context, may_be_host_failure, no_verdict};

fn run(code: Option<i32>, stdout: &str, stderr: &str) -> CheckResult {
    CheckResult {
        acceptance_id: "AC-1".into(),
        exit_code: code,
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
        quota_walk_count: 0,
        operational_error: None,
        classification: None,
    }
}

/// The host's own search path; no declared deliverables.
fn host() -> Context {
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "p.md", "digest": "d"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": [], "supplementary": []
    }))
    .unwrap();
    Context::on_host_path(&contract)
}

const RUSTC: &str = "   Compiling lake v0.1.0 (/tmp/copy)\nerror[E0308]: mismatched types\n --> src/broken.rs:1:21\n  |\n1 | pub fn f() -> u32 { \"x\" }\n  |                     ^^^ expected `u32`, found `&str`\n\nerror: could not compile `lake` (lib) due to 1 previous error\n";

#[test]
fn a_build_tool_stopping_at_a_compile_error_outside_the_checks_files_gives_no_verdict() {
    for (command, code, stderr) in [
        ("cargo test -p lake --test ingest", 101, RUSTC),
        (
            "cc -o t main.c && ./t",
            1,
            "src/util.c:3:5: error: use of undeclared identifier 'x'\n",
        ),
        (
            "npx tsc -p . && node dist/check.js",
            2,
            "src/a.ts(3,5): error TS2304: Cannot find name 'x'.\n",
        ),
        (
            "python3 -c 'import pkg'",
            1,
            "Traceback (most recent call last):\n  File \"/tmp/copy/pkg/util.py\", line 3\n    def (:\n        ^\nSyntaxError: invalid syntax\n",
        ),
    ] {
        let result = run(Some(code), "", stderr);
        let why = no_verdict(command, &result, &host()).unwrap_or_else(|| panic!("{command}"));
        assert!(why.contains("did not build"), "{why}");
        assert!(may_be_host_failure(&result), "{command}");
    }
}

#[test]
fn a_missing_tool_on_the_search_path_gives_no_verdict_whatever_stderr_says() {
    for (command, stderr) in [
        (
            "archon-issue-328-absent-tool --verify x",
            "sh: 1: archon-issue-328-absent-tool: not found\n",
        ),
        ("archon-issue-328-absent-tool --verify x 2>/dev/null", ""),
        ("/opt/archon-issue-328/bin/lint src", ""),
    ] {
        let result = run(Some(127), "", stderr);
        let why = no_verdict(command, &result, &host()).unwrap_or_else(|| panic!("{command}"));
        assert!(why.contains("not on its search path"), "{why}");
    }
    assert!(no_verdict("sleep 9", &run(None, "", ""), &host()).is_some());
}

/// (a): the deliverable not there yet is the verdict a base must give.
#[test]
fn a_program_or_script_of_the_tree_that_is_not_there_yet_is_a_verdict() {
    for command in [
        "bash scripts/new.sh 2>/dev/null",
        "sh scripts/new.sh",
        "./target/debug/tool --check",
        "cd sub && ./bin/tool",
    ] {
        for code in [126, 127] {
            let result = run(Some(code), "", "");
            assert_eq!(no_verdict(command, &result, &host()), None, "{command}");
        }
    }
}

/// (b) and (c), and the TDD case.
#[test]
fn a_compile_error_the_check_owns_or_asserts_is_a_verdict() {
    for (command, code, stdout, stderr) in [
        // (b): no build tool ran; a match of grep is not a compile error.
        (
            "! grep -rn 'error!' src",
            1,
            "src/log.rs:3:    error!(\"x\")\n",
            "",
        ),
        // TDD: the base holds the check's own test, calling missing code.
        (
            "cargo test --test tdd",
            101,
            "",
            "error[E0425]: cannot find function `missing` in crate `lake`\n --> tests/tdd.rs:3:11\n",
        ),
        (
            "cargo test --test tdd",
            101,
            "",
            "error[E0432]: unresolved import `lake::missing`\n --> tests/tdd.rs:1:5\n",
        ),
        // A file the check names.
        ("rustc --crate-type lib src/broken.rs", 1, "", RUSTC),
        // (c): the build or lint is the assertion.
        ("cargo clippy -p lake -- -D warnings", 101, "", RUSTC),
        ("cargo build -p lake", 101, "", RUSTC),
        (
            "npx tsc --noEmit",
            2,
            "",
            "src/a.ts(3,5): error TS2304: Cannot find name 'x'.\n",
        ),
        // An assertion ran beside the compile error.
        (
            "cargo test -p lake",
            101,
            "test result: FAILED. 0 passed; 1 failed\n",
            RUSTC,
        ),
        // No source location: a target or subcommand not there yet.
        (
            "cargo test --test ingest",
            101,
            "",
            "error: no test target named `ingest`\n",
        ),
        (
            "archon data status",
            2,
            "",
            "error: unrecognized subcommand 'data'\n",
        ),
        ("cargo test x", 0, "running 0 tests\n", ""),
    ] {
        let result = run(Some(code), stdout, stderr);
        assert_eq!(
            no_verdict(command, &result, &host()),
            None,
            "{command}: {stderr}"
        );
    }
}

#[test]
fn a_compile_error_in_a_declared_deliverable_is_a_verdict() {
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "p.md", "digest": "d"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": [{
            "id": "AC-2", "criterion": "c",
            "check": {"kind": "floor", "contract": {"kind": "file", "artifact_path": "src/broken.rs", "min_instances": 0}},
            "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
        }],
        "supplementary": []
    }))
    .unwrap();
    let at = Context::on_host_path(&contract);
    let result = run(Some(101), "", RUSTC);
    assert_eq!(no_verdict("cargo test -p lake", &result, &at), None);
    assert!(no_verdict("cargo test -p lake", &result, &host()).is_some());
}

#[test]
fn a_crash_in_the_checks_own_code_gives_no_verdict() {
    let command = "python3 - <<'PY'\nundefined_helper()\nPY";
    let result = run(
        Some(1),
        "",
        "Traceback (most recent call last):\n  File \"<stdin>\", line 1, in <module>\nNameError: name 'undefined_helper' is not defined\n",
    );
    let why = no_verdict(command, &result, &host());
    assert!(why.is_some_and(|why| why.contains("crashed in its own python")));
    assert!(!may_be_host_failure(&result), "a crash is the check's own");
}

/// A site whose only variables are `variables`.
fn site(variables: &[(&str, &str)]) -> Context {
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "p.md", "digest": "d"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": [], "supplementary": []
    }))
    .unwrap();
    let environment = (variables.iter())
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    Context::new(environment, &contract)
}

/// Issue 333 (CI on Windows): `bash` ran -- it printed under its own name
/// -- and exited 127 because the script it was given is not there. That is
/// its check's failure (a verdict), never a program missing from the path.
#[cfg(unix)]
#[test]
fn a_program_that_ran_and_exited_127_for_a_missing_script_is_a_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let out = std::process::Command::new("/bin/sh")
        .args(["-c", "bash scripts/three.sh"])
        .current_dir(dir.path())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    let result = run(
        out.status.code(),
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
    );
    assert_eq!(result.exit_code, Some(127), "{result:?}");
    let command = "bash scripts/three.sh";
    assert_eq!(
        no_verdict(command, &result, &site(&[("PATH", "/usr/bin:/bin")])),
        None
    );
    // Even where the resolver would not find it: what it printed proves it ran.
    assert_eq!(
        no_verdict(command, &result, &site(&[("PATH", "/nowhere")])),
        None
    );
    // A program that never ran is still missing.
    let absent = run(Some(127), "", "sh: archon-333-absent: command not found\n");
    let why = no_verdict(
        "archon-333-absent x",
        &absent,
        &site(&[("PATH", "/nowhere")]),
    );
    assert!(why.is_some_and(|why| why.contains("not on its search path")));
    let tcsh = run(Some(127), "", "archon-333-absent: Command not found.\n");
    assert!(no_verdict("archon-333-absent x", &tcsh, &site(&[("PATH", "/nowhere")])).is_some());
}

/// Issue 333: a shell that runs the check may have the check's program's
/// own name; its report that the program was not found never proves the
/// program ran. Only the program's own message does.
#[cfg(unix)]
#[test]
fn only_the_programs_own_message_proves_it_ran() {
    let nowhere = site(&[("PATH", "/nowhere")]);
    let given = "/usr/local/bin/sh";
    let cases: &[(&str, &str, bool)] = &[
        (
            "sh scripts/x.sh",
            "/bin/sh: line 1: sh: command not found",
            false,
        ),
        (
            "bash scripts/x.sh",
            "/bin/bash: line 1: bash: command not found",
            false,
        ),
        ("bash scripts/x.sh", "bash: bash: command not found", false),
        ("dash x", "/bin/dash: 1: dash: not found", false),
        (
            "/usr/local/bin/sh x",
            "/bin/sh: line 1: /usr/local/bin/sh: No such file or directory",
            false,
        ),
        (
            "bash scripts/x.sh",
            "env: bash: No such file or directory",
            false,
        ),
        ("bash scripts/x.sh", "sh: bash: not found", false),
        ("bash scripts/x.sh", "sh: 1: bash: not found", false),
        ("bash scripts/x.sh", "zsh: command not found: bash", false),
        // Its own message: it ran, and 127 is its answer.
        (
            "bash scripts/three.sh",
            "bash: scripts/three.sh: No such file or directory",
            true,
        ),
        (
            "bash scripts/three.sh",
            "/bin/bash: scripts/three.sh: No such file or directory",
            true,
        ),
        (
            "bash scripts/three.sh",
            "/bin/bash: line 0: scripts/three.sh: No such file or directory",
            true,
        ),
    ];
    for (command, stderr, verdict) in cases {
        if command.starts_with(given) && std::path::Path::new(given).exists() {
            continue;
        }
        let result = run(Some(127), "", &format!("{stderr}\n"));
        let why = no_verdict(command, &result, &nowhere);
        assert_eq!(why.is_none(), *verdict, "{command} / {stderr}: {why:?}");
        if !verdict {
            assert!(why.unwrap().contains("not on its search path"), "{stderr}");
        }
    }
}

/// A site that resolves programs by Windows's rules (or not), whatever
/// this platform is, with only `variables`.
fn site_by_rules(variables: &[(&str, &str)], windows: bool) -> Context {
    let environment = (variables.iter())
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    Context::by_rules(environment, Vec::new(), windows)
}

/// Issue 333 (CI on Windows): only the site's `Path` and `PATHEXT` decide
/// whether a program that exited 127 without naming itself was there. By
/// Windows's rules `Path` is the search path, split on `;`, and the
/// program is found with `PATHEXT`'s `.exe`: a verdict. Read as Unix reads
/// it, `Path` is no search path at all: no verdict.
#[test]
fn a_sites_path_and_pathext_alone_decide_whether_a_program_is_there() {
    let (empty, bin) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(bin.path().join("archon333tool.exe"), "").unwrap();
    let path = format!("{};{}", empty.path().display(), bin.path().display());
    let variables = [("Path", path.as_str()), ("PATHEXT", ".com;.exe")];
    let ran = run(Some(127), "", "missing input file\n");
    let command = "archon333tool --in data.csv";
    assert_eq!(
        no_verdict(command, &ran, &site_by_rules(&variables, true)),
        None
    );
    let why = no_verdict(command, &ran, &site_by_rules(&variables, false));
    assert!(why.is_some_and(|why| why.contains("not on its search path")));
    // Without `.exe` among PATHEXT's, or without the directory, it is not there.
    let no_exe = [("Path", path.as_str()), ("PATHEXT", ".com")];
    assert!(no_verdict(command, &ran, &site_by_rules(&no_exe, true)).is_some());
    let elsewhere = [
        ("Path", empty.path().to_str().unwrap()),
        ("PATHEXT", ".exe"),
    ];
    assert!(no_verdict(command, &ran, &site_by_rules(&elsewhere, true)).is_some());
    // A Windows program's own message, by its drive path, proves it ran.
    let own = run(
        Some(127),
        "",
        "C:\\Git\\bin\\archon333tool.exe: scripts/three.sh: No such file or directory\n",
    );
    assert_eq!(
        no_verdict(command, &own, &site_by_rules(&elsewhere, true)),
        None
    );
}

/// Issue 333 (CI on Windows): the site's `Path` is its search path, split
/// on `;`, and `bash` is found as `bash.exe` by the site's `PATHEXT`. The
/// program's message does not name it, so only those variables decide.
#[cfg(windows)]
#[test]
fn a_windows_sites_path_and_pathext_resolve_as_its_child_does() {
    let (empty, bin) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(bin.path().join("bash.exe"), "").unwrap();
    let path = format!("{};{}", empty.path().display(), bin.path().display());
    let unnamed = run(Some(127), "", "missing input file\n");
    let at = site(&[("Path", path.as_str()), ("PATHEXT", ".COM;.EXE")]);
    assert_eq!(no_verdict("bash scripts/three.sh", &unnamed, &at), None);
    let at = site(&[
        ("Path", empty.path().to_str().unwrap()),
        ("PATHEXT", ".COM;.EXE"),
    ]);
    assert!(no_verdict("bash scripts/three.sh", &unnamed, &at).is_some());
    let absent = run(Some(127), "", "sh: bash: command not found\n");
    assert!(no_verdict("bash scripts/three.sh", &absent, &at).is_some());
}
