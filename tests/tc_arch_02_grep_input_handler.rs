//! TC-ARCH-02 (REQ-FOR-D1): grep lint for .await on agent work in input handler.
//!
//! Runs `scripts/lint/arch-lint.sh` and asserts exit 0 on the clean tree —
//! *and* that each rule inspected something while getting there.
//!
//! Exit 0 on its own is not evidence. Two of the three rules spent months
//! scanning a marker region that had been deleted and a function-name list that
//! matched nothing, and this test passed the whole time because a script that
//! looks at zero lines finds zero violations. The counts each rule now prints
//! are the part worth asserting: a rule reporting `sites=0` has been unpointed
//! from the code it guards, whatever its exit status says.
//!
//! There is no companion test for `BEGIN`/`END INPUT_HANDLER` markers in
//! `src/main.rs`. There used to be one, `#[ignore]`d against an issue number
//! that does not exist in this repository, waiting for markers to be added
//! back. They should not be: `src/main.rs` is now a 113-line argument
//! dispatcher, the input handler lives in `src/session_loop/` and
//! `crates/archon-tui/src/event_loop/`, and a pair of comments that any refactor
//! can delete is precisely the mechanism that failed here. The region is a
//! directory list in the script, and the vacuity checks below are what keep it
//! honest.

use std::process::Command;

/// Rules the script must report on. Adding a rule to `arch-lint.sh` without
/// adding it here would leave it unasserted; leaving one here after deleting it
/// from the script fails loudly rather than quietly reducing coverage.
const EXPECTED_RULES: &[u32] = &[1, 2, 3];

#[test]
fn arch_lint_passes_on_clean_tree() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lint_script = repo_root.join("scripts/lint/arch-lint.sh");

    assert!(
        lint_script.exists(),
        "TC-ARCH-02: scripts/lint/arch-lint.sh not found at {lint_script:?}"
    );

    let output = Command::new(bash_program())
        .arg(bash_path(&lint_script))
        .current_dir(repo_root)
        .output()
        .expect("failed to execute arch-lint.sh");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "TC-ARCH-02: arch-lint.sh exited with non-zero on clean tree.\n\
         stdout: {stdout}\n\
         stderr: {stderr}"
    );

    for rule in EXPECTED_RULES {
        let report = rule_report(&stdout, *rule).unwrap_or_else(|| {
            panic!(
                "TC-ARCH-02: arch-lint.sh reported nothing for rule {rule}. Every rule must \
                 print `arch-lint: rule=N files=X sites=Y` so a rule that stopped scanning \
                 cannot hide behind exit 0.\nstdout: {stdout}\nstderr: {stderr}"
            )
        });
        assert!(
            report.files > 0,
            "TC-ARCH-02: rule {rule} scanned {} files — it has no scan target left and is \
             passing vacuously.\nstdout: {stdout}",
            report.files
        );
        assert!(
            report.sites > 0,
            "TC-ARCH-02: rule {rule} inspected {} candidate sites across {} files — the code it \
             guards has moved and the rule is passing vacuously.\nstdout: {stdout}",
            report.sites,
            report.files
        );
    }
}

/// What one rule said it inspected.
struct RuleReport {
    files: u32,
    sites: u32,
}

/// Parse `arch-lint: rule=N files=X sites=Y name=...` for a single rule.
fn rule_report(stdout: &str, rule: u32) -> Option<RuleReport> {
    let line = stdout
        .lines()
        .find(|line| line.starts_with(&format!("arch-lint: rule={rule} ")))?;
    Some(RuleReport {
        files: field(line, "files=")?,
        sites: field(line, "sites=")?,
    })
}

/// The integer following `key` in a whitespace-separated report line.
fn field(line: &str, key: &str) -> Option<u32> {
    line.split_whitespace()
        .find_map(|token| token.strip_prefix(key))
        .and_then(|value| value.parse().ok())
}

/// Locate a bash that actually exists.
///
/// Git for Windows ships `bash.exe` but its installer only adds `<git>\cmd` to
/// PATH -- the directory with `git.exe` and not `bash.exe` -- so a bare
/// `Command::new("bash")` fails with "program not found" on an otherwise
/// correctly set up machine.
fn bash_program() -> std::ffi::OsString {
    #[cfg(windows)]
    {
        // Git's bash FIRST, deliberately. A bare `bash` on Windows usually
        // resolves to the WSL launcher in System32, which runs inside
        // the Linux filesystem and cannot see `F:/...` at all -- it reports
        // "No such file or directory" for a perfectly valid Windows path.
        //
        // `where git` returns a DIFFERENT layout depending on the calling
        // shell: `<root>\cmd\git.exe` from PowerShell/cmd, but
        // `<root>\mingw64\bin\git.exe` first from Git Bash. Deriving the
        // install root by stripping exactly two components only works for
        // the former; from Git Bash it produced
        // `<root>\mingw64\bin\bash.exe`, which does not exist, silently fell
        // back to the WSL launcher, and failed the suite. Walk every
        // ancestor of every candidate instead and take the first bash.exe
        // that actually exists.
        if let Ok(output) = Command::new("where").arg("git").output() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let git = std::path::Path::new(line.trim());
                if git.as_os_str().is_empty() {
                    continue;
                }
                if let Some(bash) = bash_near(git) {
                    return bash.into_os_string();
                }
            }
        }
        "bash".into()
    }
    #[cfg(not(windows))]
    {
        "bash".into()
    }
}

/// First `bash.exe` found in any ancestor of `git`, checking both layouts Git
/// for Windows ships (`<root>\bin` and `<root>\usr\bin`).
#[cfg(windows)]
fn bash_near(git: &std::path::Path) -> Option<std::path::PathBuf> {
    const RELATIVE: [&[&str]; 2] = [&["bin", "bash.exe"], &["usr", "bin", "bash.exe"]];
    for root in git.ancestors() {
        for parts in RELATIVE {
            let mut candidate = root.to_path_buf();
            for part in parts {
                candidate.push(part);
            }
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// A path bash will accept.
///
/// Git's bash consumes backslashes as escapes, so a native Windows path arrives
/// as `F:archon-localarchon-cli...` and the script is "not found". Forward
/// slashes survive intact and Windows accepts them too.
fn bash_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
}
