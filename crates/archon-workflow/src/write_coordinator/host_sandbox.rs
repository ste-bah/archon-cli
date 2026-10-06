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
//! On Linux the same roots are sealed by a Landlock ruleset applied to the
//! child before it execs (`archon_shell::write_boundary::landlock`, which
//! documents where it is stricter). Issue-227: these verifiers REQUIRE the
//! boundary. Where none can be applied (another platform, a kernel without
//! Landlock ABI 3, a process that cannot apply one) the command is not run:
//! [`command`] answers `Err`, logged, which the callers report as the
//! environment's failure -- never the branch's, and never a silent run
//! without the boundary.

use std::path::{Path, PathBuf};

use crate::acceptance_check_environment::CommandEnvironment;

use archon_shell::write_boundary::landlock::LandlockSandbox;
use archon_shell::write_boundary::{Mechanism, SANDBOX_EXEC, SnapshotBoundary, refusal};

/// What keeps a bounded command's boundary in place after the child starts.
///
/// On Linux it holds the private temp directory the command was pointed at,
/// until the child has exited. On a host with no kernel boundary (Windows,
/// Issue-234) it holds the pre-command snapshot of the sealed roots: the caller
/// reaps the child, then calls [`BoundaryGuard::finish`], which restores and
/// names any change — the guarantee that stands in for the kernel's refusal.
#[derive(Default)]
pub(crate) struct BoundaryGuard {
    // Held only for its Drop: the private temp dir lives until the child exits.
    _landlock: Option<LandlockSandbox>,
    snapshot: Option<SnapshotBoundary>,
}

impl BoundaryGuard {
    fn landlock(sandbox: LandlockSandbox) -> Self {
        Self {
            _landlock: Some(sandbox),
            snapshot: None,
        }
    }

    fn snapshot(boundary: SnapshotBoundary) -> Self {
        Self {
            _landlock: None,
            snapshot: Some(boundary),
        }
    }

    /// Restore and name any change a host-snapshot-bounded command made to the
    /// sealed roots. `Ok(())` for a kernel-bounded or unbounded command (the
    /// kernel refused the write live, or there was nothing to snapshot), and
    /// for a snapshot that found nothing moved. Called after the child is
    /// reaped, so a restore never races the command.
    pub(crate) fn finish(self, what: &str) -> Result<(), String> {
        match self.snapshot {
            Some(snapshot) => snapshot
                .verify_restore()
                .map_err(|violation| violation.message(what)),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
thread_local! {
    static UNBOUNDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static UNAVAILABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests of what the host does when a host-run command DID change an input
/// (through an unbounded process) run their commands on this thread
/// deliberately without the boundary.
#[cfg(test)]
pub(crate) fn unbounded_for_tests(unbounded: bool) {
    UNBOUNDED.with(|cell| cell.set(unbounded));
}

/// Tests of a host that has no boundary to apply.
#[cfg(test)]
pub(crate) fn unavailable_for_tests(unavailable: bool) {
    UNAVAILABLE.with(|cell| cell.set(unavailable));
}

/// The boundary this process can apply (probed once), or why none.
fn mechanism() -> Result<Mechanism, String> {
    #[cfg(test)]
    if UNAVAILABLE.with(std::cell::Cell::get) {
        return Err("no boundary on this host (a test's stand-in)".into());
    }
    archon_shell::write_boundary::mechanism()
}

/// Whether a host-run command for a run can be bounded here.
#[cfg(test)]
pub(crate) fn available() -> bool {
    mechanism().is_ok()
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
    if let Ok(mut real) = existing.canonicalize().map(archon_shell::paths::plain) {
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
/// boundary, with `writable` re-opened; with no run (or nothing to seal) it
/// is not bounded. `Err` -- the command never runs, and that is the
/// environment's failure, never the command's -- when this host can apply no
/// boundary, or the one it can apply refuses this one (probed with a harmless
/// command first on macOS).
pub(crate) fn command(
    program: &Path,
    run_root: Option<&Path>,
    writable: &[PathBuf],
) -> Result<(std::process::Command, BoundaryGuard, CommandEnvironment), String> {
    let policy = crate::acceptance_check_environment::policy_for_run(run_root)?;
    let environment =
        CommandEnvironment::capture(policy.as_ref())?.with_remedy(if run_root.is_some() {
            crate::acceptance_check_environment::RUN_POLICY_REMEDY
        } else {
            crate::acceptance_check_environment::NO_RUN_POLICY_REMEDY
        });
    let (command, boundary) = bounded_command(program, run_root, writable, &environment)?;
    Ok((command, boundary, environment))
}

fn bounded_command(
    program: &Path,
    run_root: Option<&Path>,
    writable: &[PathBuf],
    environment: &CommandEnvironment,
) -> Result<(std::process::Command, BoundaryGuard), String> {
    let unbounded = || Ok((environment.command(program), BoundaryGuard::default()));
    let Some(run_root) = run_root else {
        return unbounded();
    };
    #[cfg(test)]
    if UNBOUNDED.with(std::cell::Cell::get) {
        return unbounded();
    }
    let project =
        crate::v2::project_artifacts::project_artifact_context_from_v2_root(&run_root.join("v2"))
            .project_root
            .map(PathBuf::from);
    let sealed = super::sealed_roots::sealed_host_roots(Some(run_root), project.as_deref(), None);
    let Some(profile) = profile(&sealed, writable) else {
        return unbounded();
    };
    let refused = |reason: &str| refusal("A host-run verifier for this run", reason);
    match mechanism().map_err(|reason| refused(&reason))? {
        Mechanism::SandboxExec => sandbox_exec(program, profile, environment)
            .map(|command| (command, BoundaryGuard::default())),
        Mechanism::Landlock { .. } => {
            let temps: Vec<PathBuf> = ["TMPDIR", "TMP", "TEMP"]
                .iter()
                .filter_map(std::env::var_os)
                .map(PathBuf::from)
                .chain([std::env::temp_dir()])
                .collect();
            let sandbox =
                LandlockSandbox::build(&sealed, writable, &temps).map_err(|r| refused(&r))?;
            let mut command = environment.command(program);
            #[cfg(target_os = "linux")]
            sandbox.install_std(&mut command);
            if let Some(dir) = sandbox.private_temp() {
                for key in ["TMPDIR", "TMP", "TEMP"] {
                    command.env(key, dir);
                }
            }
            Ok((command, BoundaryGuard::landlock(sandbox)))
        }
        // Issue-234: no kernel boundary (Windows). The command runs, and the
        // snapshot restores and names any change to a sealed root after it is
        // reaped (`BoundaryGuard::finish`). The verifier only reads, so the
        // sealed roots stay as the snapshot found them unless the environment
        // (a sibling, an unbounded process) changed them.
        Mechanism::HostSnapshot => {
            // Exclude the shared host store (`~/.archon/sessions`, `config.toml`)
            // from the snapshot: the host appends to it during a run and other
            // runs write it concurrently, so restoring it could revert their
            // writes. It stays sealed on kernel hosts; a verifier only reads.
            let mut excluded = writable.to_vec();
            excluded.extend(super::sealed_roots::user_host_stores());
            let snapshot = SnapshotBoundary::capture(&sealed, &excluded);
            Ok((
                environment.command(program),
                BoundaryGuard::snapshot(snapshot),
            ))
        }
    }
}

/// `program` under `sandbox-exec` with `profile`, once a probe shows this
/// process can apply it.
fn sandbox_exec(
    program: &Path,
    profile: String,
    environment: &CommandEnvironment,
) -> Result<std::process::Command, String> {
    let probe = archon_shell::spawn::command(SANDBOX_EXEC)
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
    let mut command = environment.command(SANDBOX_EXEC);
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
#[path = "host_verifier_environment_tests.rs"]
mod environment_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// A command under the host boundary cannot leave a sealed root changed --
    /// the run store, the acceptance evidence under the scratch parent, the
    /// project -- and can write the directory the host re-opened. On a kernel
    /// host (macOS/Linux) the write is refused live (non-zero exit); on a
    /// host-snapshot host (Windows, Issue-234) it is restored by `finish` and
    /// named. Either way nothing a verifier wrote to a sealed root persists.
    #[test]
    fn a_host_run_command_cannot_write_the_sealed_roots() {
        if !available() {
            eprintln!("skipped: no OS write boundary can be applied in this process");
            return;
        }
        let snapshot = matches!(
            archon_shell::write_boundary::mechanism(),
            Ok(archon_shell::write_boundary::Mechanism::HostSnapshot)
        );
        let shell = posix_shell_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let base = dir
            .path()
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap();
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
        // Runs `script`, then settles the boundary: on a snapshot host `finish`
        // restores any sealed change and reports it; on a kernel host it is a
        // no-op and the write was already refused. Returns the exit success and
        // whether the boundary reported a restored change.
        let run = |script: &str| -> (bool, bool) {
            let (mut command, boundary, _) =
                command(&shell, Some(&run_root), std::slice::from_ref(&worktree)).unwrap();
            let out = command
                .arg("-c")
                .arg(script)
                .current_dir(&worktree)
                .output()
                .unwrap();
            let restored = boundary.finish("the test command").is_err();
            (out.status.success(), restored)
        };
        for target in [
            evidence.clone(),
            run_root.join("state.json"),
            project.join("data.json"),
        ] {
            let (ok, restored) = run(&format!(
                "printf passed > {}",
                crate::acceptance_scratch::shell_arg(&target)
            ));
            if snapshot {
                assert!(restored, "{} change was not caught", target.display());
            } else {
                assert!(!ok, "{} was writable", target.display());
            }
        }
        // No write to a sealed root persisted, whichever mechanism applied.
        assert_eq!(std::fs::read_to_string(&evidence).unwrap(), "failed");
        assert!(!run_root.join("state.json").exists());
        assert!(!project.join("data.json").exists());
        let (ok, restored) =
            run("printf ok > own.txt && printf t > \"${TMPDIR:-/tmp}/g2-host-probe-$$\"");
        assert!(ok && !restored, "the worktree is the branch's own to write");
        assert_eq!(
            std::fs::read_to_string(worktree.join("own.txt")).unwrap(),
            "ok"
        );
    }

    /// `sh` where the host has one; on Windows the Git-for-Windows `sh`.
    fn posix_shell_for_tests() -> PathBuf {
        if cfg!(windows) {
            archon_shell::resolve_posix_shell().to_path_buf()
        } else {
            PathBuf::from("/bin/sh")
        }
    }

    /// Issue-227: a host with no boundary to apply refuses a verifier for a
    /// run -- naming the platform and why -- instead of running it unbounded;
    /// a command for no run requires none.
    #[test]
    fn a_host_with_no_boundary_refuses_a_verifier_for_a_run() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir
            .path()
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap()
            .join("project");
        let run_root = project.join(".archon/workflows/run1");
        std::fs::create_dir_all(&run_root).unwrap();
        crate::write_coordinator::project_inputs::write_test_policy(&run_root, &project, &["data"]);
        unavailable_for_tests(true);
        let refused = command(Path::new("/bin/sh"), Some(&run_root), &[]).map(|_| ());
        let unrequired = command(Path::new("/bin/sh"), None, &[]).map(|_| ());
        unavailable_for_tests(false);
        let refused = refused.unwrap_err();
        assert!(
            refused.contains(std::env::consts::OS)
                && refused.contains("NOT run")
                && refused.contains("a test's stand-in"),
            "{refused}"
        );
        assert_eq!(unrequired, Ok(()));
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
