//! The host refutes explicit existence/type/line-count claims that contradict
//! the repository; it does not write observations.

use std::path::Path;

use archon_workflow::repository_record::RepositoryTree;
use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;
use archon_write_plan::declared_paths::declared_file_paths;

use super::fences::prose_lines;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Observation {
    File { lines: usize },
    UncountedFile,
    Directory,
    Absent,
}

/// The deliverable paths a parsed task names, as written.
pub(crate) fn deliverable_paths(task: &WorkflowV2TaskUniverseTask, raw: &str) -> Vec<String> {
    let prose = prose_lines(raw).collect::<Vec<_>>().join("\n");
    let mut paths: Vec<String> = archon_workflow::task_universe::parsing::declared_files_expected_to_change_items_preserving_lines(&prose)
        .iter()
        .flat_map(|item| declared_file_paths(item))
        .collect();
    paths.extend(
        task.deliverable_contracts
            .iter()
            .map(|c| c.artifact_path.trim().to_string()),
    );
    paths.extend(
        task.shared_append_target_files
            .iter()
            .map(|p| p.trim().to_string()),
    );
    paths.retain(|path| !path.is_empty());
    paths.sort();
    paths.dedup();
    paths
}

fn is_path_shaped(token: &str) -> bool {
    if token.is_empty() || token.contains(char::is_whitespace) || token.contains("://") {
        return false;
    }
    if token.contains([
        '*', '?', '[', ']', '{', '}', '$', '<', '>', '|', '=', '"', '\'', '(', ')', ':', ',', ';',
        '`',
    ]) {
        return false;
    }
    let last = token
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(token);
    token.contains('/')
        || last
            .rsplit_once('.')
            .is_some_and(|(stem, extension)| !stem.is_empty() && !extension.is_empty())
}

/// Each occurrence of a backticked token naming `relative`, with the text
/// that follows it on its line.
#[cfg(test)]
fn mentions<'a>(tree: &RepositoryTree, line: &'a str, relative: &str) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('`') else {
            break;
        };
        let token = after_open[..close].trim();
        let after = &after_open[close + 1..];
        if !token.is_empty()
            && !token.contains(char::is_whitespace)
            && tree.relative_to_root(token).as_deref() == Some(relative)
        {
            found.push(after);
        }
        rest = after;
    }
    found
}

/// Test helper returning the first explicit observation for a path.
/// Production verification checks every claim, including repeated ones.
#[cfg(test)]
pub(crate) fn observation_for(
    tree: &RepositoryTree,
    text: &str,
    relative: &str,
) -> Option<Observation> {
    prose_lines(text)
        .flat_map(|line| mentions(tree, line, relative))
        .find_map(parse_observation)
}

/// The observation at the start of `after` (what follows the path's closing
/// backtick), or `None` when the text there is anything else.
pub(crate) fn parse_observation(after: &str) -> Option<Observation> {
    let lowered = after.to_ascii_lowercase();
    let mut rest = lowered.trim_start();
    loop {
        let trimmed = rest
            .trim_start_matches(['—', '-', '–', ':', ',', '`', '*', ')'])
            .trim_start();
        if trimmed.len() == rest.len() {
            break;
        }
        rest = trimmed;
    }
    if let Some(tail) = rest.strip_prefix("absent") {
        return ends_word(tail).then_some(Observation::Absent);
    }
    let tail = rest.strip_prefix("exists")?.trim_start();
    let inner = tail.strip_prefix('(')?;
    let close = inner.find(')')?;
    let inner = inner[..close].trim();
    if matches!(inner, "directory" | "dir") {
        return Some(Observation::Directory);
    }
    let (count, unit) = inner.split_once(char::is_whitespace)?;
    let unit = unit.trim();
    if unit != "lines" && unit != "line" {
        return None;
    }
    let lines = count.replace([',', '_'], "").parse::<usize>().ok()?;
    Some(Observation::File { lines })
}

fn ends_word(tail: &str) -> bool {
    tail.chars().next().is_none_or(|c| !c.is_alphanumeric())
}

/// Lines as the `Read` tool numbers them: `str::lines`, on bytes so a file
/// that is not UTF-8 still counts.
#[cfg(test)]
pub(crate) fn count_lines(bytes: &[u8]) -> usize {
    match bytes.last() {
        None => 0,
        Some(last) => bytes.iter().filter(|b| **b == b'\n').count() + usize::from(*last != b'\n'),
    }
}

/// One rule source for landing and lint. Only NotFound means absent.
#[cfg(test)]
pub(crate) fn observe(tree: &RepositoryTree, relative: &str) -> anyhow::Result<Observation> {
    observe_claim(tree, relative, true)
}

fn observe_claim(
    tree: &RepositoryTree,
    relative: &str,
    count: bool,
) -> anyhow::Result<Observation> {
    let path = super::repository_paths::checked_path(tree, relative)?;
    let fail = |error: std::io::Error| {
        anyhow::Error::new(error).context(format!(
            "repository observation failed for {}",
            path.display()
        ))
    };
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Observation::Absent);
        }
        Err(error) => return Err(super::repository_paths::metadata_error(relative, error)),
    };
    if metadata.is_dir() {
        return Ok(Observation::Directory);
    }
    if !metadata.is_file() {
        return Err(super::repository_paths::PathFault(format!(
            "`{relative}`: unsupported repository path file type"
        ))
        .into());
    }
    if !count {
        return Ok(Observation::UncountedFile);
    }
    use std::io::Read;
    const LIMIT: u64 = 64 * 1024 * 1024;
    let mut reader = std::fs::File::open(&path).map_err(fail)?.take(LIMIT + 1);
    let mut buffer = [0u8; 8192];
    let (mut bytes, mut lines, mut last) = (0u64, 0usize, None);
    loop {
        let count = reader.read(&mut buffer).map_err(fail)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > LIMIT {
            return Err(super::repository_paths::PathFault(format!(
                "`{relative}`: line-count claim exceeds {LIMIT} byte limit; delete the claim"
            ))
            .into());
        }
        lines += buffer[..count].iter().filter(|b| **b == b'\n').count();
        last = Some(buffer[count - 1]);
    }
    Ok(Observation::File {
        lines: lines + usize::from(last.is_some_and(|b| b != b'\n')),
    })
}

/// The blocking findings for the body `raw` of `task`, each with the
/// repository-relative path it is about.
pub(crate) fn findings_against(
    tree: &RepositoryTree,
    project_root: &Path,
    task_id: &str,
    raw: &str,
    task: &WorkflowV2TaskUniverseTask,
) -> anyhow::Result<Vec<(String, String)>> {
    // Issue-367: a file whose first line is not its frontmatter is one
    // finding, not one per deliverable the misread hides; a pure outer fence
    // (Issue-61) is read as the document inside it.
    let raw = match super::fences::task_file_shape(raw) {
        super::fences::TaskFileShape::Frontmatter => raw,
        super::fences::TaskFileShape::Wrapped(interior) => interior,
        super::fences::TaskFileShape::TextBefore(first) => {
            let text = super::fences::text_before_frontmatter_finding(first);
            return Ok(vec![(task_id.to_string(), format!("{task_id}: {text}"))]);
        }
    };
    let mut findings = Vec::new();
    let mut paths = deliverable_paths(task, raw);
    let mut observations = Vec::new();
    for (section, _) in super::repository_claims::scoped_sections(raw) {
        let parsed = super::repository_claims::extract_claims(&section);
        for line in prose_lines(&section) {
            let mut rest = line;
            while let Some((_, after)) = rest.split_once('`') {
                let Some((token, tail)) = after.split_once('`') else {
                    break;
                };
                if is_path_shaped(token)
                    && let Some(observation) = parse_observation(tail)
                {
                    let scoped = parsed.iter().find(|claim| {
                        claim.scope.is_some()
                            && archon_workflow::repository_record::normalize_relative(&claim.path)
                                == archon_workflow::repository_record::normalize_relative(token)
                    });
                    let candidates = if let Some(claim) = scoped {
                        match super::repository_claims::resolve_scoped_paths(tree, claim) {
                            Ok(paths) => paths,
                            Err(error)
                                if error
                                    .downcast_ref::<super::repository_paths::PathFault>()
                                    .is_some() =>
                            {
                                if let Some(relative) =
                                    super::repository_paths::relative_path(tree, &claim.path)?
                                    && !findings.iter().any(|(path, _)| path == &relative)
                                {
                                    findings.push((relative, format!("{task_id}: {error}")));
                                }
                                Vec::new()
                            }
                            Err(error) => return Err(error),
                        }
                    } else if token.contains('/') {
                        vec![token.to_string()]
                    } else {
                        Vec::new()
                    };
                    // No scoped reading exists: report the author spelling once.
                    let candidates = if token.contains('/')
                        && candidates.len() > 1
                        && candidates
                            .iter()
                            .map(|path| super::repository_paths::truth(tree, path))
                            .collect::<anyhow::Result<Vec<_>>>()?
                            .iter()
                            .all(|truth| truth.certainly_absent())
                    {
                        vec![token.to_string()]
                    } else {
                        candidates
                    };
                    for candidate in candidates {
                        if let Some(relative) =
                            super::repository_paths::relative_path(tree, &candidate)?
                        {
                            paths.push(candidate);
                            observations.push((relative, observation, token.to_string()));
                        }
                    }
                }
                rest = tail;
            }
        }
    }
    paths.sort();
    paths.dedup();
    let mut checked: std::collections::BTreeSet<_> =
        findings.iter().map(|(path, _)| path.clone()).collect();
    for path in paths {
        let Some(relative) =
            super::repository_paths::relative_path(tree, &path)?.filter(|r| !r.is_empty())
        else {
            continue;
        };
        if !checked.insert(relative.clone()) {
            continue;
        }
        let claims: Vec<_> = observations
            .iter()
            .filter(|(path, _, _)| path == &relative)
            .map(|(_, observation, _)| *observation)
            .collect();
        let count = claims
            .iter()
            .any(|claim| matches!(claim, Observation::File { .. }));
        let actual = match observe_claim(tree, &path, count) {
            Ok(actual) => actual,
            Err(error) => {
                if error
                    .downcast_ref::<super::repository_paths::PathFault>()
                    .is_some()
                {
                    findings.push((relative.clone(), format!("{task_id}: {error}")));
                    continue;
                }
                return Err(error);
            }
        };
        let truth = archon_workflow::repository_record::PathTruth {
            at_base: tree.exists_at_base(&relative),
            in_checkout: actual != Observation::Absent,
        };
        // A relative path present in the project but not in the checkout is
        // a project artifact (a registry, a report), not repository source.
        if !path.starts_with('/') && !truth.in_checkout && project_root.join(&relative).exists() {
            continue;
        }
        let named = observations
            .iter()
            .find(|(path, _, _)| path == &relative)
            .map_or(path.as_str(), |(_, _, named)| named.as_str());
        if truth.certainly_absent()
            && claims
                .iter()
                .any(|claim| matches!(claim, Observation::File { .. } | Observation::Directory))
            && let Some(text) = super::repository_paths::path_form_finding(tree, &relative, named)
        {
            findings.push((relative, format!("{task_id}: {text}")));
            continue;
        }
        let named = if std::path::Path::new(named).is_absolute() {
            named
        } else {
            &relative
        };
        let root = tree.root().display();
        let base = tree.base_commit();
        let checkout_dir = actual == Observation::Directory;
        // Check every explicit observation, so a generated or correct claim
        // cannot hide a later contradiction in the author's prose.
        for claimed in claims {
            let text = match claimed {
                Observation::Absent if truth.certainly_exists() => match actual {
                    Observation::File { lines } => format!(
                        "{task_id}: the body says `{named}` is absent but it exists in repository {root} at base commit {base} and in the checkout ({lines} lines); correct it to exists ({lines} lines) or delete it"
                    ),
                    Observation::UncountedFile => format!(
                        "{task_id}: the body says `{named}` is absent but it exists in repository {root} at base commit {base} and in the checkout as a file; correct the claim or delete it"
                    ),
                    _ => format!(
                        "{task_id}: the body says `{named}` is absent but it exists in repository {root} at base commit {base} and in the checkout as a directory; correct it to exists (directory) or delete it"
                    ),
                },
                Observation::File { .. } | Observation::Directory if truth.certainly_absent() => {
                    format!(
                        "{task_id}: the body says `{named}` exists but it is absent from repository {root} at base commit {base} and from the checkout; correct it to absent or delete it"
                    )
                }
                Observation::File { lines } if truth.in_checkout => {
                    if checkout_dir {
                        format!(
                            "{task_id}: the body says `{named}` exists ({lines} lines) but it is a directory in repository {root}; correct it to exists (directory) or delete it"
                        )
                    } else {
                        match actual {
                            Observation::File { lines: actual } if actual != lines => format!(
                                "{task_id}: the body says `{named}` exists ({lines} lines) but the checkout at {root} has {actual} lines; correct it to exists ({actual} lines) or delete it"
                            ),
                            _ => continue,
                        }
                    }
                }
                Observation::Directory if truth.in_checkout && !checkout_dir => {
                    format!(
                        "{task_id}: the body says `{named}` exists (directory) but it is a file in repository {root}; correct the claim or delete it"
                    )
                }
                _ => continue,
            };
            findings.push((relative.clone(), text));
            break;
        }
    }
    Ok(findings)
}

#[cfg(test)]
#[path = "repository_observations_tests.rs"]
mod tests;
