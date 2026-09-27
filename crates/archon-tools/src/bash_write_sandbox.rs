//! Issue-124: an operating-system write boundary around the shell of a write
//! branch that runs in its own isolated item worktree.
//!
//! # The gap
//!
//! A write-branch coder rewrote a live data file under the project root with
//! `python3 - <<EOF … os.replace(…)`, from its worktree, and nothing refused
//! it. The file tools resolve one path and can be judged; a shell cannot be:
//! an interpreter's inline code, a heredoc, `$(…)` and a script file all
//! write paths no reading of the command text reliably finds, so a lexical
//! refusal would be a control in appearance only (`BashTool::description_for`
//! says the same). The boundary therefore sits under the process: on macOS
//! the command runs under `sandbox-exec` with a profile the host computes
//! before the command starts, and the kernel refuses the write with `EPERM`.
//!
//! # The profile
//!
//! The sets come from the guard ([`crate::workflow_read_guard::BoundaryPaths`],
//! which documents where the host names them): writes are DENIED under every
//! sealed root — the project root, the canonical checkout, the run store — and
//! ALLOWED again, since the last matching rule wins, in the worktree, the
//! run's artifact directory and the branch's declared project artifacts. This
//! module adds what only a shell needs:
//!
//! - the worktree's own git directory (its index and `HEAD`), accepted only
//!   when it lies in `worktrees/` of a sealed checkout's git directory, since
//!   the `.git` file naming it is the agent's to edit; with git mutation
//!   allowed, that directory's `objects`, `refs` and `logs` too, never its
//!   `config` or `hooks`, which the host's own git would execute;
//! - the temp, build-cache and target directories the host itself selected
//!   for this command through the environment ([`HOST_DIR_ENV_KEYS`], the
//!   toolchain cache variables, `[tools] build_cache_env_keys`);
//! - a `literal` deny on every ancestor of a sealed root, since `subpath`
//!   does not cover an ancestor and renaming one moves the root out from
//!   under its rule. Creating entries beside an ancestor is unaffected.
//!
//! An allowed directory that contains a sealed root is dropped rather than
//! allowed: a temp directory that happened to be the project's parent would
//! otherwise re-open the whole project.
//!
//! # Why deny host roots rather than allow only the worktree
//!
//! A profile that denies every write outside a short list refuses the writes
//! ordinary toolchains make where no host can predict them — the user cache
//! and home directories of cargo, pip, npm, uv and go, `DARWIN_USER_TEMP_DIR`,
//! `/dev` — and a false refusal there costs a live run hours. The files the
//! incident damaged, and the ones a branch can damage for the whole run, are
//! the project, repository and run-store trees the host already names, so
//! those are what is sealed. Resolution is the kernel's: a symlink or a hard
//! link out of the worktree, a rename out of a sealed tree and `..` are all
//! judged on the real path.
//!
//! # What this does not cover
//!
//! Only the Bash tool's child processes are bounded. A process the command
//! asks another, unbounded process to run — a build daemon the host started,
//! an MCP server — is not. On a platform without `sandbox-exec`, or where it
//! cannot be applied (an already-sandboxed process, including a project's own
//! `sandbox-exec` call from inside a bounded command), the shell runs
//! unbounded exactly as before and a warning is logged once; the file tools
//! stay bounded by the guard either way.

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::tool::{ToolContext, ToolResult};
use crate::workflow_read_guard::{BoundaryPaths, checkout_common_dir, spellings};

pub(super) const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Environment variables through which the host points this command at a
/// directory of its own choosing; see `cache_paths::apply_shell_roots` and
/// `cargo_target_env`.
const HOST_DIR_ENV_KEYS: &[&str] = &[
    "TMPDIR",
    "TMP",
    "TEMP",
    "CARGO_TARGET_DIR",
    "ARCHON_CARGO_TARGET_DIR",
    "SCCACHE_DIR",
    "CCACHE_DIR",
];

/// The start of the note appended to a bounded command's result when a
/// write was refused; pinned by tests.
pub(super) const WRITE_BOUNDARY_NOTE_MARKER: &str = "[write boundary]";

/// What one command may and may not write, as absolute paths under every
/// spelling known (given and canonical).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WriteBoundary {
    protected: Vec<PathBuf>,
    /// Every ancestor of a protected root, denied as a `literal`.
    ancestors: Vec<PathBuf>,
    writable: Vec<PathBuf>,
    /// Declared artifact files whose `<file>.*`/`<file>~*` siblings are
    /// allowed (`regex`), and their missing parent directories (`literal`).
    siblings: Vec<PathBuf>,
    new_dirs: Vec<PathBuf>,
}

/// The boundary for this command, or `None` when the call is not an isolated
/// write branch with a host boundary, or cannot be bounded here.
pub(super) fn for_call(
    ctx: &ToolContext,
    env: &[(String, String)],
    extra_env_keys: &[String],
) -> Option<WriteBoundary> {
    let guard = ctx.workflow_read_guard.as_ref()?;
    let paths = guard.boundary_paths()?;
    let boundary = for_shell(paths, env, extra_env_keys, guard.allows_git_mutation());
    available().then_some(boundary)
}

/// The guard's sets widened by what a shell needs; see the module docs.
pub(super) fn for_shell(
    mut paths: BoundaryPaths,
    env: &[(String, String)],
    extra_env_keys: &[String],
    git_mutation: bool,
) -> WriteBoundary {
    for dir in env_dirs(env, extra_env_keys) {
        paths.allow(&dir);
    }
    for dir in git_dirs(&paths, git_mutation) {
        paths.allow(&dir);
    }
    // A declared artifact is a FILE the host allows by name. A shell writes
    // one atomically through a sibling (`<file>.tmp` then rename, `sed -i`'s
    // backup) and may first create its missing parent directories: allow
    // exactly those, never the directory the file sits in.
    let mut siblings = Vec::new();
    let mut new_dirs = Vec::new();
    for entry in &paths.writable {
        if entry.is_dir() || !paths.protected.iter().any(|root| entry.starts_with(root)) {
            continue;
        }
        push_unique(&mut siblings, entry.clone());
        let mut dir = entry.parent();
        while let Some(missing) = dir.filter(|d| d.symlink_metadata().is_err()) {
            push_unique(&mut new_dirs, missing.to_path_buf());
            dir = missing.parent();
        }
    }
    let mut ancestors = Vec::new();
    for root in &paths.protected {
        for ancestor in root.ancestors().skip(1) {
            if ancestor.to_str().is_some() {
                push_unique(&mut ancestors, ancestor.to_path_buf());
            }
        }
    }
    WriteBoundary {
        protected: paths.protected,
        ancestors,
        writable: paths.writable,
        siblings,
        new_dirs,
    }
}

impl WriteBoundary {
    /// The `sandbox-exec` profile: everything allowed, writes under the
    /// protected roots denied, the writable directories allowed again.
    ///
    /// An empty clause matches EVERY path, so neither is ever written empty:
    /// the protected list is non-empty by construction
    /// (`WorkflowReadGuard::boundary_paths` answers `None` otherwise), and an
    /// empty writable list — a worktree path that is not UTF-8 — leaves the
    /// `allow` out, which only makes the profile stricter.
    pub(super) fn profile(&self) -> String {
        let clause = |kind: &str, paths: &[PathBuf]| {
            paths
                .iter()
                .filter_map(|path| path.to_str())
                .map(|path| format!(" ({kind} \"{}\")", sbpl_escape(path)))
                .collect::<String>()
        };
        let mut profile = format!(
            "(version 1)\n(allow default)\n(deny file-write*{})\n",
            clause("subpath", &self.protected)
        );
        if !self.ancestors.is_empty() {
            let ancestors = clause("literal", &self.ancestors);
            profile.push_str(&format!("(deny file-write*{ancestors})\n"));
        }
        let mut writable = clause("subpath", &self.writable);
        writable.push_str(&clause("literal", &self.new_dirs));
        for file in self.siblings.iter().filter_map(|path| path.to_str()) {
            if !file.contains('"') {
                writable.push_str(&format!(" (regex #\"^{}[.~][^/]*$\")", regex_escape(file)));
            }
        }
        if !writable.is_empty() {
            profile.push_str(&format!("(allow file-write*{writable})\n"));
        }
        profile
    }

    /// Tell the agent what may have refused its write, where it reads: on the
    /// result of the command. The kernel's `EPERM` does not say which rule
    /// refused it, so the note says when it applies rather than that it did.
    fn annotate(&self, mut result: ToolResult) -> ToolResult {
        if !result.content.contains("Operation not permitted") {
            return result;
        }
        let list = |paths: &[PathBuf]| {
            paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        result.content.push_str(&format!(
            "\n\n{WRITE_BOUNDARY_NOTE_MARKER} This isolated write branch's shell may write only \
             in {} (and the host's own temp and cache directories); the operating system \
             refuses every other write under {}. If an \"Operation not permitted\" above is for \
             a path there, it is that boundary, not a permission problem to work around: change \
             the copy in your worktree, and report anything outside it in your envelope for the \
             host to land.",
            list(&self.writable),
            list(&self.protected)
        ));
        result
    }
}

/// [`WriteBoundary::annotate`] for a command that may not have been bounded.
pub(super) fn annotate(boundary: Option<&WriteBoundary>, result: ToolResult) -> ToolResult {
    match boundary {
        Some(boundary) => boundary.annotate(result),
        None => result,
    }
}

/// Whether `sandbox-exec` can bound a command in this process, asked once.
pub(super) fn available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let works = cfg!(target_os = "macos")
            && std::process::Command::new(SANDBOX_EXEC)
                .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
        if !works {
            tracing::warn!(
                "isolated write branches' Bash commands run WITHOUT an OS write boundary: \
                 {SANDBOX_EXEC} is unavailable or cannot be applied in this process"
            );
        }
        works
    })
}

/// Absolute directory values of the host-selected environment variables.
fn env_dirs(env: &[(String, String)], extra: &[String]) -> Vec<PathBuf> {
    let mut keys: Vec<&str> = extra.iter().map(String::as_str).collect();
    for key in HOST_DIR_ENV_KEYS
        .iter()
        .copied()
        .chain(crate::build_cache_env::toolchain_cache_env_keys())
    {
        keys.push(key);
    }
    let mut dirs = Vec::new();
    for key in keys {
        if let Some((_, value)) = env.iter().rev().find(|(name, _)| name == key) {
            let path = PathBuf::from(value.trim());
            if path.is_absolute() {
                push_unique(&mut dirs, path);
            }
        }
    }
    dirs
}

/// The worktree's own git directory and, with git mutation allowed, the
/// shared directories a commit writes. The `.git` file naming the worktree's
/// git directory is inside the worktree, so its answer is accepted only when
/// it names a single entry of `worktrees/` in a sealed checkout's git
/// directory: an edited `.git` can point at nothing else.
fn git_dirs(paths: &BoundaryPaths, git_mutation: bool) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(paths.worktree.join(".git")) else {
        return Vec::new();
    };
    let Some(named) = text.trim().strip_prefix("gitdir:").map(str::trim) else {
        return Vec::new();
    };
    let Some(gitdir) = spellings(&paths.worktree.join(named)).pop() else {
        return Vec::new();
    };
    let commons = paths
        .protected
        .iter()
        .filter_map(|root| checkout_common_dir(root))
        .flat_map(|common| spellings(&common));
    for common in commons {
        let Ok(rest) = gitdir.strip_prefix(common.join("worktrees")) else {
            continue;
        };
        // A sibling branch's gitdir is also one entry of `worktrees/`; only
        // this worktree's names this worktree back in its `gitdir` file,
        // which lives in that (sealed) directory, not in this worktree.
        if rest.components().count() != 1 || !points_back(&gitdir, &paths.worktree) {
            continue;
        }
        let mut dirs = vec![gitdir.clone()];
        if git_mutation {
            for shared in ["objects", "refs", "logs", "packed-refs"] {
                dirs.push(common.join(shared));
            }
        }
        return dirs;
    }
    Vec::new()
}

/// Whether `<gitdir>/gitdir` names `<worktree>/.git`, as git writes it for
/// the worktree the directory belongs to.
fn points_back(gitdir: &std::path::Path, worktree: &std::path::Path) -> bool {
    let Ok(named) = std::fs::read_to_string(gitdir.join("gitdir")) else {
        return false;
    };
    let named = spellings(std::path::Path::new(named.trim()));
    let ours = spellings(&worktree.join(".git"));
    named.iter().any(|path| ours.contains(path))
}

fn push_unique(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.contains(&candidate) {
        paths.push(candidate);
    }
}

/// A path as a profile regex matching itself literally.
fn regex_escape(path: &str) -> String {
    let mut out = String::new();
    for ch in path.chars() {
        if "\\.^$|?*+()[]{}".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// A profile string literal: backslash and double quote escaped.
fn sbpl_escape(path: &str) -> String {
    path.replace('\\', "\\\\").replace('"', "\\\"")
}
