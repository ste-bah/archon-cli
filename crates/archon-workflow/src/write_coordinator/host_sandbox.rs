//! Batch G2: an OS write boundary around the verifiers the host runs for a
//! write branch (its declared artifact verifiers and its declared contract
//! verifiers).
//!
//! These are the task set's own commands, and they run the branch's product
//! code on the host, outside any agent's boundary: one that wrote into the
//! project root, the canonical checkout, the run store, the acceptance
//! evidence stores or the host's transcript store and configuration could
//! change what the host later reads as a verdict. On macOS the command runs
//! under `sandbox-exec` with a profile that denies writes under every root
//! of [`super::sealed_roots::sealed_host_roots`] -- the one list every agent
//! boundary seals -- and re-opens only the directory the command works in
//! when the host names it (a branch's own worktree). Everything else a
//! toolchain writes (temp, caches, `/dev`) is untouched, so a verifier that
//! only reads keeps working.
//!
//! Where `sandbox-exec` is unavailable (another platform, or a process that
//! cannot apply one) the command runs unbounded as before: the project-input
//! tripwire around it is then the only guard, which is a stated leftover.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

#[cfg(test)]
thread_local! {
    static UNBOUNDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests of what the host does when a host-run command DID change an input
/// (off macOS, or through an unbounded process) run their commands on this
/// thread without the boundary.
#[cfg(test)]
pub(crate) fn unbounded_for_tests(unbounded: bool) {
    UNBOUNDED.with(|cell| cell.set(unbounded));
}

/// Whether a profile can be applied in this process (probed once).
fn available() -> bool {
    #[cfg(test)]
    if UNBOUNDED.with(std::cell::Cell::get) {
        return false;
    }
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        cfg!(target_os = "macos")
            && std::process::Command::new(SANDBOX_EXEC)
                .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
    })
}

/// `path` as given and with its longest existing prefix resolved: the kernel
/// judges the real path, and `/var` is `/private/var` on macOS.
fn spellings(path: &Path) -> Vec<PathBuf> {
    let mut out = vec![path.to_path_buf()];
    let mut existing = path;
    let mut rest = Vec::new();
    while existing.symlink_metadata().is_err() {
        let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
            return out;
        };
        rest.push(name.to_os_string());
        existing = parent;
    }
    if let Ok(mut real) = existing.canonicalize() {
        real.extend(rest.iter().rev());
        if !out.contains(&real) {
            out.push(real);
        }
    }
    out
}

fn quoted(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The sandbox profile sealing `sealed` and re-opening `writable` (dropped
/// when it would re-open a sealed root). `None` with nothing to seal.
pub(crate) fn profile(sealed: &[PathBuf], writable: &[PathBuf]) -> Option<String> {
    let sealed: Vec<PathBuf> = sealed.iter().flat_map(|root| spellings(root)).collect();
    if sealed.is_empty() {
        return None;
    }
    let mut rules = String::from("(version 1)\n(allow default)\n");
    for root in &sealed {
        rules.push_str(&format!("(deny file-write* (subpath {}))\n", quoted(root)));
        // Renaming an ancestor would move the root out from under its rule.
        let mut ancestor = root.parent();
        while let Some(dir) = ancestor.filter(|dir| dir.parent().is_some()) {
            rules.push_str(&format!("(deny file-write* (literal {}))\n", quoted(dir)));
            ancestor = dir.parent();
        }
    }
    for dir in writable.iter().flat_map(|dir| spellings(dir)) {
        if sealed.iter().any(|root| root.starts_with(&dir)) {
            continue;
        }
        rules.push_str(&format!("(allow file-write* (subpath {}))\n", quoted(&dir)));
    }
    Some(rules)
}

/// A command running `program` for the run at `run_root`, under the host
/// boundary where one can be applied, with `writable` re-opened. `Err` when
/// the boundary this process can apply refuses the profile itself (probed
/// with a harmless command first): the command never runs, and that is the
/// environment's failure, never the command's.
pub(crate) fn command(
    program: &Path,
    run_root: Option<&Path>,
    writable: &[PathBuf],
) -> Result<std::process::Command, String> {
    let Some(run_root) = run_root.filter(|_| available()) else {
        return Ok(std::process::Command::new(program));
    };
    let project =
        crate::v2::project_artifacts::project_artifact_context_from_v2_root(&run_root.join("v2"))
            .project_root
            .map(PathBuf::from);
    let sealed = super::sealed_roots::sealed_host_roots(Some(run_root), project.as_deref(), None);
    let Some(profile) = profile(&sealed, writable) else {
        return Ok(std::process::Command::new(program));
    };
    let probe = std::process::Command::new(SANDBOX_EXEC)
        .arg("-p")
        .arg(&profile)
        .arg("/usr/bin/true")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|error| format!("the host's write boundary could not be started: {error}"))?;
    if !probe.status.success() {
        return Err(format!(
            "the host's write boundary could not be applied: {}",
            String::from_utf8_lossy(&probe.stderr).trim()
        ));
    }
    let mut command = std::process::Command::new(SANDBOX_EXEC);
    command.arg("-p").arg(profile).arg(program);
    Ok(command)
}

/// What a host-run verifier working in `cwd` for a branch may write: `cwd`
/// itself (dropped by the profile when it is a host root), its toolchain
/// dependency and cache directories (a worktree's are links into the
/// canonical checkout, judged at their real path), and what the host
/// stamped writable for the branch (`input`'s `_write_boundary`).
pub(crate) fn verifier_writable(cwd: &Path, input: Option<&serde_json::Value>) -> Vec<PathBuf> {
    let mut writable = vec![cwd.to_path_buf()];
    writable.extend(
        (crate::v2::write::SHARED_TOOLCHAIN_DIRS.iter())
            .map(|name| cwd.join(name))
            .filter(|dir| dir.exists()),
    );
    if let Some((_, stamped)) = input.and_then(crate::agent_dispatch_port::write_boundary) {
        writable.extend(stamped.into_iter().map(PathBuf::from));
    }
    writable
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command under the host boundary cannot write a sealed root -- the
    /// run store, the acceptance evidence under the scratch parent -- and
    /// can write the directory the host re-opened and its temp directory.
    #[test]
    fn a_host_run_command_cannot_write_the_sealed_roots() {
        if !available() {
            eprintln!("skipped: sandbox-exec cannot be applied in this process");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let project = base.join("project");
        let run_root = project.join(".archon/workflows/run1");
        let worktree = run_root.join("v2/worktrees/impl/impl-0");
        let evidence = base.join("observations/acceptance-evidence-1/observation.json");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(evidence.parent().unwrap()).unwrap();
        std::fs::write(&evidence, "failed").unwrap();
        crate::write_coordinator::project_inputs::write_test_policy(&run_root, &project, &["data"]);
        let policy = run_root.join("v2/generated-metadata.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&policy).unwrap()).unwrap();
        value["observer_snapshot"]["native_execution"]["policy"]["scratch_parent"] =
            serde_json::json!(base.join("observations"));
        std::fs::write(&policy, serde_json::to_vec(&value).unwrap()).unwrap();
        let run = |script: &str| {
            command(
                Path::new("/bin/sh"),
                Some(&run_root),
                std::slice::from_ref(&worktree),
            )
            .unwrap()
            .arg("-c")
            .arg(script)
            .current_dir(&worktree)
            .output()
            .unwrap()
        };
        for target in [
            evidence.clone(),
            run_root.join("state.json"),
            project.join("data.json"),
        ] {
            let out = run(&format!("printf passed > {}", target.display()));
            assert!(!out.status.success(), "{} was writable", target.display());
        }
        assert_eq!(std::fs::read_to_string(&evidence).unwrap(), "failed");
        let out = run("printf ok > own.txt && printf t > \"${TMPDIR:-/tmp}/g2-host-probe-$$\"");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join("own.txt")).unwrap(),
            "ok"
        );
    }

    /// A verifier may write what the branch's own agent may: its working
    /// directory, its toolchain directories, and what the host stamped.
    #[test]
    fn a_verifier_is_granted_what_its_branch_may_write() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        let stamped = dir.path().join("wt.project-artifacts/docs");
        let input = serde_json::json!({
            crate::agent_dispatch_port::WRITE_BOUNDARY_INPUT_KEY:
                {"sealed": ["/p"], "writable": [stamped]},
        });
        let writable = verifier_writable(dir.path(), Some(&input));
        for expected in [dir.path().to_path_buf(), dir.path().join("target"), stamped] {
            assert!(writable.contains(&expected), "{expected:?} in {writable:?}");
        }
        assert!(!writable.contains(&dir.path().join("node_modules")));
    }
}
