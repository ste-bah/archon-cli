//! Generic input mutation (A4): prove that a check passing on the
//! pre-implementation tree can fail. Move aside text-derived data paths,
//! excluding command mechanics, manifests, generated outputs, siblings and
//! paths outside the copied check tree. Run the check
//! in a subshell, then restore moved paths in reverse order even when its
//! `trap` or `exit` runs. Nonce markers prevent forged results. Each
//! mutation uses a hermetic copy; a live-root guard prevents moving live data.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, AcceptanceCriterion, TrustedCwd,
};
use paths::path_tokens;

use super::executed_text;

#[path = "workflow_acceptance_executability_mutation_operational.rs"]
mod operational;
use operational::operational_words;

#[path = "workflow_acceptance_executability_mutation_markers.rs"]
mod marker_protocol;
pub(super) use marker_protocol::Markers;

#[path = "workflow_acceptance_executability_mutation_wrapper_script.rs"]
mod wrapper_script;
#[cfg(test)]
pub(super) use wrapper_script::wrap;

#[path = "workflow_acceptance_executability_mutation_inline.rs"]
mod inline;

#[path = "workflow_acceptance_executability_mutation_paths.rs"]
mod paths;

#[path = "workflow_acceptance_executability_mutation_repository_alias.rs"]
mod repository_alias;

/// The exit status of a run stopped at the live-root guard.
pub(super) const GUARD_STATUS: i32 = 97;
/// What every finding for a check that passes before any implementation
/// says, so a stalled re-author can escalate it.
pub(crate) const CANNOT_FAIL: &str = "passed on the pre-implementation tree";

/// Build and package manifests: a check fails without them for no reason
/// its criterion gives.
const MANIFESTS: [&str; 26] = [
    "Cargo.toml",
    "Cargo.lock",
    "package.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
    "go.mod",
    "go.sum",
    "Makefile",
    "makefile",
    "GNUmakefile",
    "CMakeLists.txt",
    "build.gradle",
    "build.gradle.kts",
    "pom.xml",
    "Gemfile",
    "Gemfile.lock",
    "composer.json",
    "deno.json",
    "tsconfig.json",
    "meson.build",
    "Package.swift",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExclusionReason {
    MissingAtRoot,
    ManifestOrSetup,
    SiblingFromRoot,
    RepositoryCopyAlias,
    RepositoryAliasUnverified,
    CheckWritesPathInText,
    CheckRecreatedPathObserved,
    OutsideCopiedTree,
    CheckMechanics,
}

impl ExclusionReason {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::MissingAtRoot => "missing at the selected root",
            Self::ManifestOrSetup => "build manifest or setup file",
            Self::SiblingFromRoot => "sibling tree seen from this root; not movable",
            Self::RepositoryCopyAlias => {
                "repository sibling alias resolves inside the copied repository; movable there"
            }
            Self::RepositoryAliasUnverified => {
                "repository sibling alias target could not be verified in the site copy; not movable"
            }
            Self::CheckWritesPathInText => {
                "a command in the check writes this path (seen in the check text), so it is not a pre-existing input"
            }
            Self::CheckRecreatedPathObserved => {
                "the check recreated this path while running (observed)"
            }
            Self::OutsideCopiedTree => "outside the copied check tree",
            Self::CheckMechanics => "check mechanics or command output",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CandidateDiagnostic {
    pub(super) path: String,
    pub(super) move_path: Option<String>,
    pub(super) move_root: Option<String>,
    pub(super) exclusion: Option<ExclusionReason>,
}

#[derive(Default)]
pub(super) struct InputDiscovery {
    pub(super) eligible: Vec<String>,
    pub(super) candidates: Vec<CandidateDiagnostic>,
}

#[cfg(test)]
pub(super) fn input_discovery(
    entry: &AcceptanceCriterion,
    project: &Path,
    repository: &Path,
) -> InputDiscovery {
    input_discovery_for_check(entry, project, repository, None)
}

pub(super) fn input_discovery_for_check(
    entry: &AcceptanceCriterion,
    project: &Path,
    repository: &Path,
    copy_layout: Option<&super::hermetic::CopyLayout>,
) -> InputDiscovery {
    let text = executed_text(entry).map_or_else(String::new, |(_, text)| text.to_string());
    let (raw_commands, bodies) = paths::lex(&text);
    let cwd = match &entry.check {
        AcceptanceCheck::Command {
            cwd: TrustedCwd::RepoRoot,
            ..
        } => repository,
        _ => project,
    };
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let (commands, bases, _) = paths::effective_directories(&raw_commands, &cwd);
    let expanded_text = commands
        .iter()
        .map(|words| words.join(" "))
        .collect::<Vec<_>>()
        .join("; ");
    let siblings =
        archon_workflow::acceptance_sibling_paths::discover(&cwd, &[text.clone(), expanded_text]);
    let message_words: BTreeSet<String> = commands
        .iter()
        .filter_map(|words| {
            let program = words
                .first()
                .and_then(|word| Path::new(word).file_name())?
                .to_string_lossy();
            matches!(program.as_ref(), "echo" | "printf" | "fail" | "die")
                .then(|| words.iter().skip(1).cloned().collect::<Vec<_>>())
        })
        .flatten()
        .collect();
    let written_words: BTreeSet<String> = commands
        .iter()
        .flat_map(|words| {
            words
                .windows(2)
                .filter(|pair| matches!(pair[0].as_str(), ">" | ">>"))
                .map(|pair| pair[1].clone())
        })
        .collect();
    let excluded: Vec<String> = (operational_words(&commands).iter())
        .flat_map(|word| path_tokens(word))
        .chain(message_words.iter().flat_map(|word| path_tokens(word)))
        .chain(written_words.iter().flat_map(|word| path_tokens(word)))
        .collect();
    let bases = paths::command_directories(&commands, bases);
    let inline_programs = inline::program_positions(&commands);
    let structured_program_texts: BTreeSet<String> = inline::programs(&commands)
        .into_iter()
        .map(|(_, _, program)| program.trim_end_matches('\n').to_string())
        .chain(
            bodies
                .iter()
                .map(|(_, body)| body.trim_end_matches('\n').to_string()),
        )
        .collect();
    let mut words: Vec<(String, Option<PathBuf>)> = commands
        .iter()
        .enumerate()
        .flat_map(|(index, words)| {
            let base = bases.get(index).cloned().flatten();
            let inline_programs = &inline_programs;
            words
                .iter()
                .enumerate()
                .filter(move |(word_index, word)| {
                    !inline_programs.contains(&(index, *word_index))
                        && !matches!(
                            word.as_str(),
                            paths::SUBSHELL_OPEN | paths::SUBSHELL_CLOSE | "export"
                        )
                        && !word.starts_with('\0')
                        && !word.starts_with('$')
                        && !word.contains(['$', '`'])
                        && !word.split_once('=').is_some_and(|(name, _)| {
                            !name.is_empty()
                                && name
                                    .bytes()
                                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                        })
                })
                .map(|(_, word)| word.clone())
                .map(move |word| (word, base.clone()))
        })
        .collect();
    for (index, _, program) in inline::programs(&commands) {
        words.push((program, bases.get(index).cloned().flatten()));
    }
    for (index, body) in bodies {
        words.push((body, bases.get(index).cloned().flatten()));
    }
    // Keep sibling discovery and mutation input discovery on the same view of
    // inline programs and here-doc bodies. The richer shell lexer above still
    // supplies per-command working directories; this shared pass catches
    // program text carried through syntax it cannot associate with a command.
    words.extend(
        archon_workflow::acceptance_sibling_paths::program_texts(&text)
            .into_iter()
            // The structured pass already has the effective directory for
            // these programs. Re-adding them here would resolve their paths
            // from the initial cwd as well, potentially naming project files
            // that do not exist there when the command has changed directory.
            .filter(|program| !structured_program_texts.contains(program.trim_end_matches('\n')))
            .map(|program| (program, Some(cwd.clone()))),
    );
    if let AcceptanceCheck::Floor { contract } = &entry.check {
        words.push((contract.artifact_path.clone(), Some(cwd.clone())));
    }
    let mut seen = BTreeSet::new();
    let candidates: Vec<CandidateDiagnostic> = (words.iter())
        .flat_map(|(word, base)| {
            path_tokens(word)
                .into_iter()
                .map(move |token| (token, base.clone()))
        })
        .filter_map(|(token, base)| {
            let path = Path::new(&token);
            let Some(base) = base else {
                return None;
            };
            let resolved_path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                base.join(path)
            };
            let exists = super::hermetic::siblings::exact_path_exists(&resolved_path);
            let direct_copy_move =
                repository_alias::direct_copy_repository_input(path, &cwd, repository, copy_layout);
            let direct_copy = direct_copy_move.is_some();
            let shaped = token.contains('/') || exists;
            let resolved = resolved_path.canonicalize().ok();
            let escapes_through_symlink = resolved
                .as_ref()
                .is_some_and(|resolved| !resolved.starts_with(&cwd))
                && has_symlink_component(&base, path);
            let sibling_root = resolved.as_ref().and_then(|resolved| {
                siblings.iter().find(|sibling| {
                    sibling
                        .source
                        .canonicalize()
                        .is_ok_and(|source| resolved.starts_with(source))
                })
            });
            let repository_copy_alias = direct_copy_move.is_some()
                || sibling_root.is_some_and(|sibling| {
                    repository_alias::is_copy_alias(sibling, repository, copy_layout, &entry.id)
                });
            let repository_sibling = sibling_root
                .is_some_and(|sibling| repository_alias::is_repository_source(sibling, repository));
            let sibling = (exists && sibling_root.is_some()) || direct_copy;
            let mapped_sibling = sibling_root.and_then(|sibling| {
                let resolved = resolved.as_ref()?;
                let source = sibling.source.canonicalize().ok()?;
                let within = resolved.strip_prefix(&source).ok()?;
                Some(
                    paths::relative_path(&cwd, &source.join(within))
                        .to_string_lossy()
                        .into_owned(),
                )
            });
            let manifest_sibling = commands
                .iter()
                .filter(|words| {
                    words
                        .iter()
                        .flat_map(|word| path_tokens(word))
                        .any(|candidate| candidate == token)
                })
                .find_map(|words| {
                    super::hermetic::siblings::manifest_relative_input(
                        &token,
                        &words.join(" "),
                        &base,
                    )
                });
            let sibling = sibling || manifest_sibling.is_some();
            let manifest = path
                .file_name()
                .is_some_and(|name| MANIFESTS.contains(&name.to_string_lossy().as_ref()));
            // Neither what runs the check nor a directory that holds it.
            let runs = (excluded.iter()).any(|word| Path::new(word).starts_with(path));
            let has_manifest_sibling = manifest_sibling.is_some();
            let mapped = mapped_sibling.or(manifest_sibling).unwrap_or_else(|| {
                paths::relative_path(&cwd, &resolved_path)
                    .to_string_lossy()
                    .into_owned()
            });
            let canonical_move = direct_copy_move.or_else(|| {
                if repository_copy_alias {
                    sibling_root.and_then(|sibling| {
                        let resolved = resolved.as_ref()?;
                        repository_alias::canonical_move_path(
                            sibling,
                            resolved,
                            repository,
                            copy_layout,
                            &entry.id,
                        )
                    })
                } else {
                    None
                }
            });
            let exclusion = if written_words.contains(&token) {
                Some(ExclusionReason::CheckWritesPathInText)
            } else if manifest {
                Some(ExclusionReason::ManifestOrSetup)
            } else if escapes_through_symlink {
                Some(ExclusionReason::OutsideCopiedTree)
            } else if sibling && repository_copy_alias && canonical_move.is_some() {
                Some(ExclusionReason::RepositoryCopyAlias)
            } else if sibling && repository_sibling {
                Some(ExclusionReason::RepositoryAliasUnverified)
            } else if sibling {
                Some(ExclusionReason::SiblingFromRoot)
            } else if runs {
                Some(ExclusionReason::CheckMechanics)
            } else if mapped.starts_with("../") {
                Some(ExclusionReason::OutsideCopiedTree)
            } else if !exists && !has_manifest_sibling && canonical_move.is_none() {
                Some(ExclusionReason::MissingAtRoot)
            } else {
                None
            };
            (shaped
                || exists
                || canonical_move.is_some()
                || has_manifest_sibling
                || written_words.contains(&token))
            .then_some(CandidateDiagnostic {
                path: mapped,
                move_path: canonical_move.as_ref().map(|(path, _)| path.clone()),
                move_root: canonical_move.map(|(_, root)| root),
                exclusion,
            })
        })
        .filter(|candidate| seen.insert(candidate.path.clone()))
        .collect();
    let eligible = candidates
        .iter()
        // #471 diagnostics report missing paths, but path-shaped inputs
        // remain eligible just as they were before the diagnostic pass.
        .filter(|candidate| {
            candidate.exclusion.is_none()
                || candidate.exclusion == Some(ExclusionReason::RepositoryCopyAlias)
                || candidate.exclusion == Some(ExclusionReason::MissingAtRoot)
        })
        .map(|candidate| candidate.path.clone())
        .collect();
    InputDiscovery {
        eligible,
        candidates,
    }
}

fn has_symlink_component(base: &Path, path: &Path) -> bool {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut current = PathBuf::new();
    for component in candidate.components() {
        current.push(component.as_os_str());
        if std::fs::symlink_metadata(&current)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return true;
        }
    }
    false
}

/// `contract` with each of `inputs`' entries' executed text wrapped by
/// [`wrap`]. Nothing else changes, so each wrapped entry resolves exactly as
/// its original would.
pub(super) fn mutated(
    contract: &AcceptanceContract,
    inputs: &BTreeMap<String, Vec<String>>,
    move_paths: &BTreeMap<String, Vec<String>>,
    move_roots: &BTreeMap<String, Vec<Option<String>>>,
    live_roots: &[&Path],
    markers: &Markers,
) -> AcceptanceContract {
    let mut mutated = contract.clone();
    for entry in (mutated.acceptance.iter_mut()).chain(&mut mutated.supplementary) {
        let Some(names) = inputs.get(&entry.id) else {
            continue;
        };
        let paths = move_paths.get(&entry.id).map(Vec::as_slice).unwrap_or(&[]);
        let roots = move_roots.get(&entry.id).map(Vec::as_slice).unwrap_or(&[]);
        match &mut entry.check {
            AcceptanceCheck::Command { command, .. } => {
                *command = wrapper_script::wrap_with_paths(
                    command, names, paths, roots, live_roots, markers,
                );
            }
            AcceptanceCheck::Floor { contract } => {
                if let Some(command) = contract.typed_verifier_command.as_mut() {
                    *command = wrapper_script::wrap_with_paths(
                        command, names, paths, roots, live_roots, markers,
                    );
                }
            }
        }
    }
    mutated
}

#[cfg(test)]
#[path = "workflow_acceptance_executability_mutation_test_helpers.rs"]
mod test_helpers;
#[cfg(test)]
pub(crate) use test_helpers::named_inputs;

#[cfg(all(test, unix))]
pub(super) use test_helpers::run_with_host_mutation;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_mutation_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_mutation_variable_tests.rs"]
mod variable_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_mutation_inline_tests.rs"]
mod inline_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_mutation_diagnostics_tests.rs"]
mod diagnostics_tests;
