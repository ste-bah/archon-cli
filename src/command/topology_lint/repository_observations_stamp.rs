//! Deterministic, host-owned facts kept outside author prose and frontmatter.
use std::path::Path;

use anyhow::{Context, Result};
use archon_workflow::repository_record::{RepositoryTree, git_head, read_repository_record};
use archon_workflow::task_universe::parsing::parse_task_file;

use super::repository_observations::{Observation, deliverable_paths, observe};

const START: &str = "\n<!-- host-repository-observations -->\n";
const END: &str = "<!-- /host-repository-observations -->\n";

pub(crate) fn stamp(task_root: &Path, task_file: &Path, candidate: &str) -> Result<String> {
    let result = stamp_inner(task_root, task_file, candidate);
    if let Err(error) = &result {
        eprintln!(
            "stamping repository observations failed for {}: {error:#}",
            task_file.display()
        );
    }
    result
}

fn stamp_inner(task_root: &Path, task_file: &Path, candidate: &str) -> Result<String> {
    // Sets created before repository recording keep their existing behavior.
    let Some(record) = read_repository_record(task_root)? else {
        return Ok(candidate.to_string());
    };
    let tree = RepositoryTree::load(&record).context("loading repository for observation stamp")?;
    let revision =
        git_head(tree.root()).context("reading repository revision for observation stamp")?;
    let body = if let Some((body, generated)) = candidate.rsplit_once(START) {
        anyhow::ensure!(
            generated.ends_with(END),
            "incomplete host repository observation block"
        );
        body
    } else {
        candidate
    };
    let task = parse_task_file(task_file, body)
        .context("parsing body for repository observation stamp")?;
    let mut entries = std::collections::BTreeMap::new();
    for path in deliverable_paths(&task, body) {
        let Some(relative) = tree.relative_to_root(&path).filter(|p| !p.is_empty()) else {
            continue;
        };
        let observation = match observe(&tree, &relative)? {
            Observation::Absent => serde_json::json!({"kind": "absent"}),
            Observation::Directory => serde_json::json!({"kind": "directory"}),
            Observation::File { lines } => serde_json::json!({"kind": "file", "lines": lines}),
        };
        entries.insert(relative, observation);
    }
    let facts = serde_json::json!({
        "repository_root": record.repository_root,
        "base_commit": record.base_commit,
        "checkout_revision": revision,
        "paths": entries,
    });
    Ok(format!(
        "{}{START}```json\n{}\n```\n{END}",
        body.trim_end_matches('\n'),
        serde_json::to_string_pretty(&facts)?
    ))
}
