//! What a run's agents may read beyond the directory they work in, and the
//! launch-time proof that they can (Issue-56).
//!
//! A workflow may work in one directory and read code in another: the fixed
//! decomposition runs its authors in the project directory, which holds the
//! PRD and the task root, and tells them to ground every claim in the
//! repository the spec names as `target_repository_root`. Issue-55 put that
//! root in the prompt; nothing put it in the tool sandbox, so every `Read`,
//! `Glob` and `Grep` of the repository was refused as "outside allowed
//! directories" and the authors wrote bodies around the refusal for four
//! hours. Two things stop that here: [`read_roots`] names the directories a
//! spec's repository root adds to every agent's allowed roots, and
//! [`require_agent_read`] asks the real client, through the real guard, to
//! admit the repository root before the first agent is dispatched.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use archon_workflow::WorkflowLlmClient;

/// The directories a run's agents may read beyond `cwd`: the spec's
/// `target_repository_root` when it names somewhere other than `cwd` itself.
///
/// Keyed on the spec's field, not on the kind of run, so any workflow whose
/// spec carries a repository root gets it. `cwd` is dropped rather than
/// listed twice because it is already the working directory.
pub(crate) fn read_roots(cwd: &Path, target_repository_root: Option<&str>) -> Vec<PathBuf> {
    let Some(root) = target_repository_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
    else {
        return Vec::new();
    };
    if same_directory(cwd, &root) {
        return Vec::new();
    }
    vec![root]
}

/// `roots` plus the run's author-context directory (Issue 288), created now
/// so the guard can resolve it. Author prompts name files there by exact
/// path. It sits under `.archon`, a host-excluded name; as a read root of its
/// own the exclusions apply only below it, where there are none, and every
/// sibling under `.archon` stays refused.
pub(crate) fn with_author_context(mut roots: Vec<PathBuf>, run_dir: &Path) -> Result<Vec<PathBuf>> {
    let dir = archon_workflow::v2::script::author_context::author_context_dir(run_dir);
    std::fs::create_dir_all(&dir).map_err(|error| {
        anyhow!(
            "creating author context directory {}: {error}",
            dir.display()
        )
    })?;
    roots.push(dir);
    Ok(roots)
}

fn same_directory(a: &Path, b: &Path) -> bool {
    let canonical = |path: &Path| {
        std::fs::canonicalize(path)
            .map(archon_shell::paths::plain)
            .unwrap_or_else(|_| path.to_path_buf())
    };
    canonical(a) == canonical(b)
}

/// Refuse to go on unless the agents `client` dispatches can read `path`.
///
/// The probe runs the guard the agents' own tools consult against the context
/// they inherit, so a refusal here is the refusal the first author would have
/// met — and it surfaces in under a second at launch, with the guard's own
/// text, instead of hours later in a body written around it. A client with no
/// tool sandbox has nothing to refuse and passes.
pub(crate) fn require_agent_read(
    client: &dyn WorkflowLlmClient,
    path: &Path,
    who: &str,
) -> Result<()> {
    match client.probe_agent_read(path) {
        None => {
            tracing::debug!(path = %path.display(), who, "client runs no tool sandbox; read access not probed");
            Ok(())
        }
        Some(Ok(())) => Ok(()),
        Some(Err(refusal)) => Err(anyhow!(
            "{who} could not read {}: the tool sandbox refused it before any agent was dispatched ({refusal}); \
             the run's read roots must include the repository root",
            path.display()
        )),
    }
}

/// The file the launch check writes in a run's author-context directory and
/// asks the authors' guard to admit. Not a content digest, so it can never
/// collide with a context file.
const AUTHOR_CONTEXT_PROBE: &str = "launch-read-probe.txt";

/// The launch proof for a run's authors (Issue 288): they can read `root`
/// (the repository, when the spec names one), and they can read the run's
/// author-context directory under the same host exclusions every author call
/// runs with. Author prompts name earlier work by exact path in that
/// directory, so a composition that refused it would leave every author
/// reading around a refusal; this ends the launch instead, with its cause.
pub(crate) fn require_author_reads(
    client: &dyn WorkflowLlmClient,
    who: &str,
    root: Option<&Path>,
    run_dir: &Path,
) -> Result<()> {
    if let Some(root) = root {
        require_agent_read(client, root, who)?;
    }
    let dir = archon_workflow::v2::script::author_context::author_context_dir(run_dir);
    let probe = dir.join(AUTHOR_CONTEXT_PROBE);
    std::fs::write(&probe, b"author context read probe\n").map_err(|error| {
        anyhow!(
            "writing the author context read probe {}: {error}",
            probe.display()
        )
    })?;
    let verdict = archon_tools::read_boundary::sync_scope(
        archon_leann::language::default_exclude_patterns(),
        || client.probe_agent_read(&probe),
    );
    match verdict {
        None | Some(Ok(())) => Ok(()),
        Some(Err(refusal)) => Err(anyhow!(
            "{who} could not read the run's author-context directory {}: the tool sandbox \
             refused {} before any agent was dispatched ({refusal}); author prompts name \
             earlier work by exact path there, so the run's read roots must include that \
             directory",
            dir.display(),
            probe.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::{WorkflowAgentOutcome, WorkflowResult};

    struct ProbeClient(Option<Result<(), String>>);

    #[async_trait::async_trait]
    impl WorkflowLlmClient for ProbeClient {
        /// Scripted replies stand for one continued session (#241).
        async fn continue_agent(
            &self,
            call: archon_workflow::WorkflowAgentCall,
        ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
            self.run_agent(call).await
        }

        fn probe_agent_read(&self, _path: &Path) -> Option<Result<(), String>> {
            self.0.clone()
        }

        async fn send_message(
            &self,
            _messages: Vec<serde_json::Value>,
            _system: Vec<serde_json::Value>,
            _tools: Vec<serde_json::Value>,
            _model: &str,
        ) -> WorkflowResult<WorkflowAgentOutcome> {
            unreachable!("the probe never completes")
        }
    }

    #[test]
    fn read_roots_name_the_repository_unless_it_is_the_working_directory() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        let repo_text = repo.display().to_string();

        assert_eq!(read_roots(&project, Some(&repo_text)), vec![repo.clone()]);
        assert!(
            read_roots(&repo, Some(&repo_text)).is_empty(),
            "the working dir is already readable"
        );
        assert!(read_roots(&project, None).is_empty());
        assert!(read_roots(&project, Some("  ")).is_empty());
    }

    /// Issue 288: the author-context directory is created and added last,
    /// beside whatever roots the run already had.
    #[test]
    fn the_author_context_directory_is_created_and_added_as_a_read_root() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join(".archon/workflows/wf-1");
        let repo = temp.path().join("repo");
        let roots = with_author_context(vec![repo.clone()], &run_dir).unwrap();
        let context = run_dir.join("author-context");
        assert_eq!(roots, vec![repo, context.clone()]);
        assert!(context.is_dir());
        assert_eq!(
            with_author_context(Vec::new(), &run_dir).unwrap(),
            vec![context]
        );
        let blocker = temp.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        assert!(
            with_author_context(Vec::new(), &blocker).is_err(),
            "an uncreatable directory is an error"
        );
    }

    #[test]
    fn a_refused_probe_aborts_with_the_guard_text_and_an_admitted_one_passes() {
        let repo = Path::new("/somewhere/repo");
        let refused = ProbeClient(Some(Err(
            "Path '/somewhere/repo' is outside allowed directories: /somewhere/project".into(),
        )));
        let error = require_agent_read(&refused, repo, "the decomposition authors").unwrap_err();
        let text = format!("{error:#}");
        assert!(
            text.contains("the decomposition authors could not read /somewhere/repo"),
            "{text}"
        );
        assert!(
            text.contains("outside allowed directories: /somewhere/project"),
            "{text}"
        );

        assert!(require_agent_read(&ProbeClient(Some(Ok(()))), repo, "authors").is_ok());
        assert!(require_agent_read(&ProbeClient(None), repo, "authors").is_ok());
    }

    /// Issue 288: an author-context directory the authors cannot read ends
    /// the launch with a clear cause; a readable one passes.
    #[test]
    fn an_unreadable_author_context_directory_ends_the_launch() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join(".archon/workflows/wf-1");
        with_author_context(Vec::new(), &run_dir).unwrap();
        let refused = ProbeClient(Some(Err("Path is in a host-excluded subtree".into())));
        let text = require_author_reads(&refused, "the authors", None, &run_dir)
            .unwrap_err()
            .to_string();
        assert!(
            text.contains("the authors could not read the run's author-context directory"),
            "{text}"
        );
        assert!(
            text.contains("read roots must include that directory"),
            "{text}"
        );
        assert!(
            run_dir
                .join("author-context")
                .join(AUTHOR_CONTEXT_PROBE)
                .is_file()
        );
        let admitted = ProbeClient(Some(Ok(())));
        assert!(require_author_reads(&admitted, "the authors", None, &run_dir).is_ok());
        let repo = Path::new("/somewhere/repo");
        let error = require_author_reads(&refused, "the authors", Some(repo), &run_dir);
        assert!(
            error
                .unwrap_err()
                .to_string()
                .contains("could not read /somewhere/repo")
        );
    }
}
