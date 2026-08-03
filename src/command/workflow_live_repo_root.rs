use std::fs;
use std::path::{Path, PathBuf};

use archon_workflow::task_universe::WorkflowV2TaskUniverse;

const REPOSITORY_MARKERS: &[(&str, i32)] = &[
    (".git", 120),
    ("Cargo.toml", 80),
    ("package.json", 80),
    ("pyproject.toml", 80),
    ("go.mod", 80),
    ("pom.xml", 70),
    ("build.gradle", 70),
    ("settings.gradle", 70),
    ("composer.json", 70),
    ("Gemfile", 70),
    ("mix.exs", 70),
    ("deno.json", 70),
    ("deno.jsonc", 70),
    ("CMakeLists.txt", 60),
    ("Makefile", 50),
];

#[derive(Debug, Clone, Eq, PartialEq)]
struct RepositoryCandidate {
    path: PathBuf,
    score: i32,
    distance: usize,
}

pub(super) fn infer_target_repository_root(
    task: &str,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> Option<String> {
    explicit_target_repository_root(task).or_else(|| {
        task_universe
            .and_then(infer_from_task_universe)
            .map(|path| path.display().to_string())
    })
}

fn explicit_target_repository_root(task: &str) -> Option<String> {
    [
        "against the repository ",
        "against repository ",
        "repository root ",
        "repository ",
        "repo ",
    ]
    .into_iter()
    .find_map(|marker| path_after_marker(task, marker))
}

fn infer_from_task_universe(universe: &WorkflowV2TaskUniverse) -> Option<PathBuf> {
    let mut best = None;
    for path in universe_paths(universe) {
        scan_source_path(&mut best, Path::new(&path));
    }
    best.map(|candidate: RepositoryCandidate| candidate.path)
}

fn universe_paths(universe: &WorkflowV2TaskUniverse) -> Vec<String> {
    universe
        .source_roots
        .iter()
        .cloned()
        .chain(universe.tasks.iter().map(|task| task.source_path.clone()))
        .collect()
}

fn scan_source_path(best: &mut Option<RepositoryCandidate>, path: &Path) {
    let base = source_base(path);
    for (distance, ancestor) in base.ancestors().take(6).enumerate() {
        consider_candidate(best, ancestor, distance);
        scan_direct_children(best, ancestor, distance + 1);
    }
}

fn source_base(path: &Path) -> &Path {
    if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    }
}

fn scan_direct_children(best: &mut Option<RepositoryCandidate>, dir: &Path, distance: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok).take(200) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            consider_candidate(best, &entry.path(), distance);
        }
    }
}

fn consider_candidate(best: &mut Option<RepositoryCandidate>, path: &Path, distance: usize) {
    let Some(score) = repository_score(path, distance) else {
        return;
    };
    let candidate = RepositoryCandidate {
        path: path.to_path_buf(),
        score,
        distance,
    };
    if best
        .as_ref()
        .is_none_or(|current| better(&candidate, current))
    {
        *best = Some(candidate);
    }
}

fn repository_score(path: &Path, distance: usize) -> Option<i32> {
    let marker_score = REPOSITORY_MARKERS
        .iter()
        .filter(|(marker, _)| path.join(marker).exists())
        .map(|(_, score)| *score)
        .max()?;
    Some(marker_score - distance as i32)
}

fn better(candidate: &RepositoryCandidate, current: &RepositoryCandidate) -> bool {
    candidate.score > current.score
        || (candidate.score == current.score && candidate.distance < current.distance)
        || (candidate.score == current.score
            && candidate.distance == current.distance
            && candidate.path < current.path)
}

fn path_after_marker(task: &str, marker: &str) -> Option<String> {
    let (_, rest) = task.split_once(marker)?;
    let path = rest
        .split_whitespace()
        .next()?
        .trim_matches(|ch: char| matches!(ch, ',' | ';' | ':' | '.' | ')' | ']'));
    looks_like_filesystem_path(path).then(|| path.to_string())
}

fn looks_like_filesystem_path(path: &str) -> bool {
    path.starts_with('/') || path.starts_with("~/") || is_windows_absolute_path(path)
}

fn is_windows_absolute_path(path: &str) -> bool {
    let mut chars = path.chars();
    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(drive), Some(':'), Some('\\' | '/')) if drive.is_ascii_alphabetic()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;

    #[test]
    fn explicit_repository_text_wins_over_inferred_repo() {
        let temp = tempfile::tempdir().unwrap();
        let explicit = temp.path().join("explicit");
        let inferred = temp.path().join("inferred");
        fs::create_dir_all(explicit.join(".git")).unwrap();
        fs::create_dir_all(inferred.join(".git")).unwrap();
        let universe = universe_for(temp.path().join("project/tasks"));

        let task = format!("implement against repository {}", explicit.display());

        assert_eq!(
            infer_target_repository_root(&task, Some(&universe)),
            Some(explicit.display().to_string())
        );
    }

    #[test]
    fn infers_sibling_repository_from_task_pack_root() {
        let temp = tempfile::tempdir().unwrap();
        let tasks = temp.path().join("project/tasks");
        let repo = temp.path().join("source-repo");
        fs::create_dir_all(&tasks).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();

        let universe = universe_for(tasks);

        assert_eq!(
            infer_target_repository_root("implement decomposed PRD", Some(&universe)),
            Some(repo.display().to_string())
        );
    }

    #[test]
    fn manifest_repository_is_fallback_when_git_is_absent() {
        let temp = tempfile::tempdir().unwrap();
        let tasks = temp.path().join("project/tasks");
        let repo = temp.path().join("source-repo");
        fs::create_dir_all(&tasks).unwrap();
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("pyproject.toml"), "[project]\nname = 'demo'\n").unwrap();

        let universe = universe_for(tasks);

        assert_eq!(
            infer_target_repository_root("implement decomposed PRD", Some(&universe)),
            Some(repo.display().to_string())
        );
    }

    #[test]
    fn runtime_project_artifact_root_without_repo_marker_is_not_selected() {
        let temp = tempfile::tempdir().unwrap();
        let tasks = temp.path().join("project/tasks");
        fs::create_dir_all(&tasks).unwrap();
        fs::create_dir_all(temp.path().join("project/.archon/workflows")).unwrap();

        let universe = universe_for(tasks);

        assert_eq!(
            infer_target_repository_root("implement", Some(&universe)),
            None
        );
    }

    #[test]
    fn closer_repo_with_workflow_artifacts_wins_over_farther_repo() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("workspace");
        let tasks = parent.join("project/tasks");
        let repo = parent.join("source-repo");
        let farther = temp.path().join("aaa-unrelated");
        fs::create_dir_all(&tasks).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join(".archon/workflows")).unwrap();
        fs::create_dir_all(farther.join(".git")).unwrap();

        let universe = universe_for(tasks);

        assert_eq!(
            infer_target_repository_root("implement decomposed PRD", Some(&universe)),
            Some(repo.display().to_string())
        );
    }

    fn universe_for(tasks: PathBuf) -> WorkflowV2TaskUniverse {
        WorkflowV2TaskUniverse {
            schema_version: "test".to_string(),
            source_roots: vec![tasks.display().to_string()],
            tasks: vec![WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-ABC-001".to_string(),
                aliases: Vec::new(),
                source_path: tasks.join("TASK-ABC-001.md").display().to_string(),
                dependency_ids: Vec::new(),
                title: None,
                artifact_requirements: Vec::new(),
                ..Default::default()
            }],
        }
    }
}
