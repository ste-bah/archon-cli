//! A check that names a live root by its absolute path (Issue 366).
//!
//! A check starts from its working directory in whichever tree the host
//! runs it in. A freeze proves it in a copy (the scratch observation, or the
//! probe's own hermetic copy); an acceptance round runs it in the scratch
//! copy when `[workflow.acceptance_execution]` is configured, and otherwise
//! in the live checkout itself. A command that names a live root by its
//! absolute path would read or change the live tree from a copy too. The
//! freeze probe refuses such a check unrun, and both acceptance authors (the
//! decomposition's author step and the host re-author) refuse it in the same
//! words, so the author repairs it before any judge. All use this one rule,
//! and both authors are told it in one text ([`check_path_rule`]).
//!
//! A root is named when its text occurs in the command and the next
//! character does not continue a path component (`[A-Za-z0-9._-]`): the root
//! itself, a path under it, or the root followed by any other character (a
//! quote, a glob, a variable, a `printf` directive). A longer sibling name
//! that only starts with the root's text (`<root>-other`, `<root>.bak`) is a
//! different path and is not the root, unless its path leads back under the
//! root (`<root>-x/../<name>`). Every absolute path in the text is also read
//! lexically (`//`, `/./` and `x/..` resolved), so a spelling of a root with
//! redundant separators is still that root. Only absolute roots are matched:
//! a relative root would match nearly any text.
//!
//! On Windows a root has two spellings: the plain one (`C:\x`,
//! `\\server\share\x`) and the verbatim one that `canonicalize` gives
//! (`\\?\C:\x`, `\\?\UNC\server\share\x`). Both are forms of it. There
//! the text is compared without case and `/` is read as `\`, as Windows
//! resolves a path ([`Spelling`]).

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf, Prefix};

use crate::command::acceptance_scratch_policy::NativeBinding;

/// What every acceptance author is told before it writes a check, with the
/// repository's and the project's absolute paths: where a check can run, and
/// the relative-path rule. One text for the decomposition author
/// (`checkPathRule` in `workflow_decompose_v1_acceptance.js`, held equal to
/// this by test) and the host re-author.
pub(crate) fn check_path_rule(repository: &str, project: &str) -> String {
    format!(
        "Where a check runs: a check starts in a POSIX shell whose working directory is the project root when cwd is project_root (a floor's typed_verifier_command always starts there) or the repository root when cwd is repo_root, of whichever tree the host runs it in. Before acceptance the host proves each check in a disposable copy: a clone of the code repository at its committed HEAD (uncommitted changes are not in it) and the project files the host is configured to copy, which may be fewer than the project holds. An acceptance round runs the same check in such a copy or, when no isolated acceptance execution is configured, in the live repository and project themselves. So a check must not change, delete or reset anything outside its own temporary files (build output its own commands produce excepted). Choose the cwd whose root holds the files the command reads: repo_root for repository source and tests; never reach one root from the other with ..: the roots do not sit the same way in every copy.\nIn every check command, name every path relative to the check's working directory; never write the repository's or the project's absolute path ({repository}, {project}) or any path under them: those paths are for your reading only. The host refuses a check that names either root by its absolute path and never runs it."
    )
}

/// The repository a task set was decomposed against (its
/// `repository.lock`), else the project for a set that predates the record.
/// A lock that exists but cannot be believed (unreadable, malformed, of
/// another schema, or naming a root that is not a directory) is an error,
/// as the record's own contract says: the true repository root is then
/// unknown, and no check is run and no author told a root from a guess
/// (Issue 366, N1).
pub(crate) fn recorded_repository(project: &Path, tasks_root: &Path) -> Result<PathBuf, String> {
    use archon_workflow::repository_record::{read_repository_record, repository_record_path};
    let lock = repository_record_path(tasks_root);
    // The lock and its repair lead: a binding fault's text can be cut short.
    let unknown = |why: String| {
        format!(
            "restore {} as its decomposition wrote it, or remove the task set and decompose again, then re-run: the task set's repository root is unknown ({why})",
            lock.display()
        )
    };
    match read_repository_record(tasks_root) {
        Ok(None) => Ok(project.to_path_buf()),
        Ok(Some(record)) if Path::new(&record.repository_root).is_dir() => {
            Ok(PathBuf::from(record.repository_root))
        }
        Ok(Some(record)) => Err(unknown(format!(
            "{} records repository {}, which is not a directory",
            lock.display(),
            record.repository_root
        ))),
        Err(error) => Err(unknown(error.to_string())),
    }
}

/// The repository a freeze proves a task set's checks against: the scratch
/// policy's (`binding`) when one is configured, else the recorded one. The
/// freeze probe and the author step both take it from here (L4). A lock
/// that cannot be believed is an error under a policy too: the freeze pins
/// its checks' sources from that lock when it publishes.
pub(crate) fn freeze_repository(
    project: &Path,
    tasks_root: &Path,
    binding: Option<&NativeBinding>,
) -> Result<PathBuf, String> {
    let recorded = recorded_repository(project, tasks_root)?;
    Ok(binding.map_or(recorded, |binding| binding.policy.repository.clone()))
}

/// The live roots of a task set as its freeze probe sees them: the project
/// and [`freeze_repository`] under the policy configured now. A policy that
/// cannot be captured adds nothing (the freeze then runs nothing); a lock
/// that cannot be believed is an error.
pub(crate) fn task_set_roots(project: &Path, tasks_root: &Path) -> Result<Vec<PathBuf>, String> {
    let binding = crate::command::acceptance_scratch_policy::capture(project, tasks_root);
    let binding = binding.ok().flatten();
    Ok(vec![
        freeze_repository(project, tasks_root, binding.as_ref())?,
        project.to_path_buf(),
    ])
}

/// The forms of `roots` check text could name: each absolute root as given
/// and its canonical form, each in its plain and its verbatim spelling,
/// without trailing separators. A relative root, a filesystem root (`/`,
/// `C:\`, `\\?\C:\`, `\\server\share\`), a drive-relative path (`D:`)
/// and empty text have no form: none is a live root to match.
pub(crate) fn root_forms<'a>(roots: impl IntoIterator<Item = &'a Path>) -> Vec<PathBuf> {
    let mut spellings = Vec::new();
    for root in roots.into_iter().filter(|root| root.has_root()) {
        spellings.push(root.to_path_buf());
        if let Ok(canonical) = root.canonicalize() {
            spellings.push(canonical);
        }
    }
    let mut forms: Vec<PathBuf> = (spellings.into_iter())
        .flat_map(|spelling| {
            let plain = archon_shell::paths::plain(spelling.clone());
            let verbatim = verbatim(&plain);
            [Some(spelling), Some(plain), verbatim]
        })
        .flatten()
        // Rebuilt from its components: no trailing separator, one separator.
        .map(|form| form.components().collect::<PathBuf>())
        .filter(|form| below_a_filesystem_root(form))
        .collect();
    forms.sort();
    forms.dedup();
    forms
}

/// Whether `form` names a path under a filesystem root: it has a root and
/// a component past its prefix and root directory. `D:` (drive-relative)
/// has no root; `/`, `C:\` and `\\server\share\` are the root itself.
fn below_a_filesystem_root(form: &Path) -> bool {
    form.has_root()
        && (form.components()).any(|c| !matches!(c, Component::Prefix(_) | Component::RootDir))
}

/// The verbatim spelling of a plain drive or UNC path (`C:\x` is
/// `\\?\C:\x`, `\\server\share\x` is `\\?\UNC\server\share\x`); any
/// other path (every POSIX path) has none.
fn verbatim(plain: &Path) -> Option<PathBuf> {
    let mut components = plain.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return None;
    };
    let mut text = OsString::from(r"\\?\");
    match prefix.kind() {
        Prefix::Disk(letter) => text.push(format!("{}:", char::from(letter))),
        Prefix::UNC(server, share) => {
            text.push(r"UNC\");
            text.push(server);
            text.push(r"\");
            text.push(share);
        }
        _ => return None,
    }
    let mut path = PathBuf::from(text);
    path.extend(components);
    Some(path)
}

/// `roots` as absolute paths, or why one is not: the author-step validator
/// refuses to apply the rule with a relative root.
pub(crate) fn absolute_roots(roots: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    match roots.iter().find(|root| !root.has_root()) {
        Some(root) => Err(format!(
            "live root {} is not absolute; only absolute canonical roots are matched",
            root.display()
        )),
        None => Ok(roots.to_vec()),
    }
}

/// Whether `c` continues a path component (so the root's text is only the
/// start of a longer name).
fn continues_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// How check text spells a path on a host: POSIX compares it exactly;
/// Windows compares it without case and reads `/` as `\`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spelling {
    Posix,
    Windows,
}

impl Spelling {
    /// The spelling of the host this binary runs on.
    const HOST: Spelling = if cfg!(windows) {
        Spelling::Windows
    } else {
        Spelling::Posix
    };

    /// `text` as this spelling compares it.
    fn fold(self, text: &str) -> String {
        match self {
            Spelling::Posix => text.to_string(),
            Spelling::Windows => text.to_lowercase().replace('/', "\\"),
        }
    }

    /// Whether `c` ends a path token: whitespace or shell syntax. On
    /// Windows `\` separates components, `:` ends a drive letter and `?`
    /// marks a verbatim prefix (`\\?\`): none of them ends a path there.
    fn ends_token(self, c: char) -> bool {
        let syntax = c.is_whitespace() || "'\"`;|&<>()$=:,{}[]*?!#%\\".contains(c);
        syntax && !(self == Spelling::Windows && matches!(c, '\\' | ':' | '?'))
    }
}

/// `token` (an absolute path) with repeated separators, `.` and `x/..`
/// resolved: the path the host would reach. A `..` never climbs past the
/// root.
fn lexical(token: &str) -> Option<PathBuf> {
    let path = Path::new(token);
    if !path.has_root() {
        return None;
    }
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    resolved.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    resolved.pop();
                }
            }
            component => resolved.push(component),
        }
    }
    Some(resolved)
}

/// The first of `forms` (from [`root_forms`]) that `text` names, in the
/// host's [`Spelling`].
pub(crate) fn named_root<'a>(text: &str, forms: &'a [PathBuf]) -> Option<&'a PathBuf> {
    named_root_in(Spelling::HOST, text, forms)
}

/// The first of `forms` that `text` names, both compared in `spelling`.
fn named_root_in<'a>(spelling: Spelling, text: &str, forms: &'a [PathBuf]) -> Option<&'a PathBuf> {
    let text = spelling.fold(text);
    let ends = |c: char| spelling.ends_token(c);
    let paths: Vec<PathBuf> = text.split(ends).filter_map(lexical).collect();
    forms.iter().find(|form| {
        let root = spelling.fold(&form.to_string_lossy());
        let form = Path::new(&root);
        let named_at = |at: usize| match text[at + root.len()..].chars().next() {
            // A sibling name, unless its path leads back under the root.
            Some(next) if continues_name(next) => {
                let token = text[at..].split(ends).next().unwrap_or_default();
                lexical(token).is_some_and(|path| path.starts_with(form))
            }
            _ => true,
        };
        !root.is_empty()
            && (text.match_indices(&root).any(|(at, _)| named_at(at))
                || paths.iter().any(|path| path.starts_with(form)))
    })
}

/// The finding for check `id` that names live root `root`: the one text
/// the freeze and both authors give.
pub(crate) fn live_root_finding(id: &str, root: &Path) -> String {
    format!(
        "check '{id}': it names the live root {} by its absolute path, so no hermetic copy can keep it off the live tree and the host never runs it; name every path relative to the check's working directory",
        root.display()
    )
}

/// The text the host executes for an authored entry's `check` value: a
/// command's text, or a floor's non-blank verifier (as `executed_text`).
pub(crate) fn executed_check_text(check: &serde_json::Value) -> Option<&str> {
    match check.get("kind")?.as_str()? {
        "command" => check.get("command")?.as_str(),
        "floor" => (check
            .get("contract")?
            .get("typed_verifier_command")?
            .as_str())
        .filter(|command| !command.trim().is_empty()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "workflow_acceptance_live_root_tests.rs"]
pub(crate) mod tests;
