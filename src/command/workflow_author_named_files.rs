use crate::command::workflow_task_set::{executability::Baseline, judge, reauthor::author};
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, JudgeDecision, JudgeVerdict, TrustedCwd,
};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

struct Roots {
    project: PathBuf,
    repository: PathBuf,
    tasks: PathBuf,
}

fn files(
    entry: &str,
    roots: &Roots,
    notes: &str,
    mut remaining: usize,
    cache: &mut judge::ReauthorPrefetchCache,
) -> Result<String, String> {
    let entry: Value = serde_json::from_str(entry).map_err(|error| error.to_string())?;
    let notes: Vec<String> = serde_json::from_str(notes).map_err(|error| error.to_string())?;
    let criterion = entry["criterion"].as_str().unwrap_or_default().to_owned();
    let check = entry
        .get("check")
        .cloned()
        .and_then(|check| serde_json::from_value(check).ok())
        .unwrap_or_else(|| AcceptanceCheck::Command {
            command: criterion.clone(),
            cwd: TrustedCwd::RepoRoot,
        });
    let frozen = AcceptanceCriterion {
        id: entry["id"].as_str().unwrap_or_default().to_owned(),
        criterion,
        check,
        gap_permitted: false,
        covers: Vec::new(),
        judgment: JudgeVerdict {
            verdict: JudgeDecision::Refuted,
            reason: String::new(),
            counterexample: String::new(),
            host_call_id: String::new(),
            sampling: None,
        },
    };
    let inputs = author::prefetch_inputs(&frozen, &notes, &roots.project, &roots.repository);
    let repository_commit =
        Baseline::for_task_set(&roots.repository, &roots.tasks).map(|baseline| baseline.commit);
    let roots = judge::JudgeRoots {
        project: Some(roots.project.clone()),
        repository: Some(roots.repository.clone()),
        task_root: Some(roots.tasks.clone()),
        repository_commit,
    };
    let files =
        judge::named_files_for_reauthor_cached(&frozen, &roots, &inputs, &mut remaining, cache);
    serde_json::to_string(&files).map_err(|error| error.to_string())
}

pub(super) fn install(ctx: &rquickjs::Ctx<'_>, args: Option<&Value>) -> rquickjs::Result<()> {
    // Capture the host's launch roots; script mutations cannot redirect reads.
    let roots = args.and_then(|args| {
        let roots = Roots {
            project: PathBuf::from(args.get("projectRoot")?.as_str()?),
            repository: PathBuf::from(args.get("repositoryRoot")?.as_str()?),
            tasks: PathBuf::from(args.get("taskRoot")?.as_str()?),
        };
        (roots.project.is_absolute() && roots.repository.is_absolute() && roots.tasks.is_absolute())
            .then_some(roots)
    });
    let cache = Mutex::new(judge::ReauthorPrefetchCache::default());
    ctx.globals().set(
        "__archonNamedFiles",
        rquickjs::function::Func::from(move |entry: String, notes: String, remaining: usize| {
            let Some(roots) = roots.as_ref() else {
                return Ok("[]".to_owned());
            };
            // Discovery and file resolution are native work. Keep their CPU
            // off the script thread without resetting its execution guards.
            #[cfg(test)]
            let body_reads = judge::body_read_observer();
            let result = std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        #[cfg(test)]
                        judge::set_body_read_observer(body_reads);
                        let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
                        files(&entry, roots, &notes, remaining, &mut cache)
                    })
                    .join()
                    .unwrap_or_else(|error| std::panic::resume_unwind(error))
            });
            result.map_err(|message| {
                rquickjs::Error::new_from_js_message("named files", "string", message)
            })
        }),
    )
}

#[cfg(test)]
#[path = "workflow_author_named_files_tests.rs"]
mod tests;
