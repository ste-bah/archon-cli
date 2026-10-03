//! Process-environment isolation for this binary's tests (Issue 280).
//!
//! The test harness runs every test of the binary in one process, many at a
//! time. A test that sets or removes a process environment variable changes
//! it under every test running beside it. Live: a freeze probe keyed its
//! saved verdicts by the environment it read, an unrelated test set a
//! variable between the save and the retry, and the retry found no verdict
//! and ran every check again.
//!
//! So a test that changes the environment runs alone, in a child process of
//! this test binary ([`run_alone!`]), and changes it only through [`set_var`]
//! and [`remove_var`], which refuse to run anywhere else.
//! `env_writes_go_through_this_module` keeps every other spelling out of the
//! binary's source.

use std::ffi::OsStr;
use std::process::Command;

/// Set in a child process that runs one test alone: the test's full name.
pub(crate) const CHILD: &str = "ARCHON_TEST_ENV_CHILD";

/// Run test `name` of `module` (its `module_path!()`) alone in a child
/// process of this test binary. True in the parent, once the child passed;
/// false in the child, which then runs the test's body.
pub(crate) fn run_test_alone(module: &str, name: &str) -> bool {
    let module = module.split_once("::").map_or("", |(_, rest)| rest);
    let test = format!("{module}::{name}");
    if std::env::var(CHILD).as_deref() == Ok(test.as_str()) {
        return false;
    }
    let output = Command::new(std::env::current_exe().expect("the test binary"))
        .args([
            test.as_str(),
            "--exact",
            "--include-ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, &test)
        .output()
        .expect("run the test in a child process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{test} failed in its child process ({:?})\nstdout:\n{stdout}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

/// Run the enclosing test alone in a child process ([`run_test_alone`]); the
/// parent returns here once the child passed.
macro_rules! run_alone {
    ($name:ident) => {
        if $crate::test_env::run_test_alone(module_path!(), stringify!($name)) {
            return;
        }
    };
}
pub(crate) use run_alone;

fn assert_alone() {
    assert!(
        std::env::var_os(CHILD).is_some(),
        "a test changes the process environment only when it runs alone: \
         start it with `crate::test_env::run_alone!(<test name>)`"
    );
}

/// [`std::env::set_var`], in a process running one test alone.
///
/// # Safety
///
/// As for [`std::env::set_var`]. No other test shares this process
/// (asserted); the test's own threads must not read the environment
/// meanwhile.
pub(crate) unsafe fn set_var(key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
    assert_alone();
    // SAFETY: the caller upholds `std::env::set_var`'s contract.
    unsafe { std::env::set_var(key, value) }
}

/// [`std::env::remove_var`], in a process running one test alone.
///
/// # Safety
///
/// As for [`set_var`].
pub(crate) unsafe fn remove_var(key: impl AsRef<OsStr>) {
    assert_alone();
    // SAFETY: the caller upholds `std::env::remove_var`'s contract.
    unsafe { std::env::remove_var(key) }
}

#[cfg(test)]
mod tests {
    /// The only places in this binary that change the process environment:
    /// this module, and startup before any thread is spawned.
    const ALLOWED: &[&str] = &["src/test_env.rs", "src/main_bootstrap.rs"];

    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn env_writes_go_through_this_module() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        sources(&root.join("src"), &mut files);
        // `env::set_var(` not preceded by an identifier character: the
        // standard library's, however it is imported, never this module's.
        let needles = [
            concat!("env::", "set_var("),
            concat!("env::", "remove_var("),
        ];
        let writes = |line: &str| {
            needles.iter().any(|needle| {
                line.match_indices(needle).any(|(at, _)| {
                    !line[..at]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_')
                })
            })
        };
        let offenders: Vec<String> = files
            .iter()
            .filter(|path| {
                let relative = path.strip_prefix(root).unwrap().to_string_lossy();
                !ALLOWED.contains(&relative.as_ref())
            })
            .filter_map(|path| {
                let text = std::fs::read_to_string(path).ok()?;
                text.lines()
                    .position(&writes)
                    .map(|line| format!("{}:{}", path.display(), line + 1))
            })
            .collect();
        assert!(
            offenders.is_empty(),
            "change the process environment through crate::test_env, in a test \
             started with run_alone!: {offenders:#?}"
        );
    }
}
