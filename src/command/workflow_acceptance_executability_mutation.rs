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

#[path = "workflow_acceptance_executability_mutation_markers.rs"]
mod marker_protocol;
pub(super) use marker_protocol::Markers;

#[path = "workflow_acceptance_executability_mutation_wrapper_script.rs"]
mod wrapper_script;
pub(super) use wrapper_script::wrap;

#[path = "workflow_acceptance_executability_mutation_inline.rs"]
mod inline;

#[path = "workflow_acceptance_executability_mutation_paths.rs"]
mod paths;

/// The exit status of a run stopped at the live-root guard.
pub(super) const GUARD_STATUS: i32 = 97;
/// What every finding for a check that passes before any implementation
/// says, so a stalled re-author can escalate it.
pub(crate) const CANNOT_FAIL: &str = "passed on the pre-implementation tree";

/// Programs whose first operand is the script they run.
const INTERPRETERS: [&str; 18] = [
    "sh", "bash", "zsh", "dash", "ksh", "python", "python2", "python3", "node", "deno", "bun",
    "ruby", "perl", "php", "Rscript", "lua", "tclsh", "pwsh",
];
/// Words that precede the command word without being it.
const PREFIXES: [&str; 22] = [
    "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "for", "case", "esac",
    "in", "!", "exec", "env", "time", "command", "nohup", "xargs", "timeout", "nice",
];
/// Options whose value is a directory or manifest the command works from.
const LOCATION_FLAGS: [&str; 6] = [
    "-C",
    "--manifest-path",
    "--directory",
    "--cwd",
    "--project",
    "--prefix",
];
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

/// The words of `text` that make the check run (see the module docs).
fn operational_words(commands: &[Vec<String>]) -> BTreeSet<String> {
    let mut excluded = BTreeSet::new();
    for words in commands {
        if words
            .iter()
            .any(|word| matches!(word.as_str(), paths::SUBSHELL_OPEN | paths::SUBSHELL_CLOSE))
        {
            continue;
        }
        let mut rest = words
            .iter()
            .skip_while(|word| {
                PREFIXES.contains(&word.as_str())
                    || matches!(word.as_str(), "\0and" | "\0or")
                    || word.starts_with('\0')
                    || word.starts_with('-')
                    || (word.contains('=') && !word.starts_with('='))
            })
            .peekable();
        let Some(command) = rest.next() else {
            continue;
        };
        excluded.insert(command.clone());
        let program = Path::new(command)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let operands: Vec<&String> = rest.collect();
        if program == "cd" || program == "pushd" {
            excluded.extend(operands.iter().take(1).map(|word| word.to_string()));
        }
        if INTERPRETERS.contains(&program.as_str()) {
            for word in &operands {
                let inline = word.starts_with('-')
                    && !word.starts_with("--")
                    && (word.contains('c') || word.contains('e'));
                if inline || word.as_str() == "-" || word.as_str() == "-s" {
                    break;
                }
                if !word.starts_with('-') {
                    excluded.insert(word.to_string());
                    break;
                }
            }
        }
        for pair in operands.windows(2) {
            if LOCATION_FLAGS.contains(&pair[0].as_str()) {
                excluded.insert(pair[1].to_string());
            }
        }
        for word in &operands {
            if let Some((flag, value)) = word.split_once('=')
                && LOCATION_FLAGS.contains(&flag)
            {
                excluded.insert(value.to_string());
            }
        }
    }
    excluded
}

/// Candidate data paths `entry` names, relative to its working directory.
#[cfg(test)]
pub(super) fn named_inputs(
    entry: &AcceptanceCriterion,
    project: &Path,
    repository: &Path,
) -> Vec<String> {
    input_discovery(entry, project, repository).eligible
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExclusionReason {
    MissingAtRoot,
    ManifestOrSetup,
    SiblingFromRoot,
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
    pub(super) exclusion: Option<ExclusionReason>,
}

#[derive(Default)]
pub(super) struct InputDiscovery {
    pub(super) eligible: Vec<String>,
    pub(super) candidates: Vec<CandidateDiagnostic>,
}

pub(super) fn input_discovery(
    entry: &AcceptanceCriterion,
    project: &Path,
    repository: &Path,
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
            let shaped = token.contains('/') || exists;
            let resolved = resolved_path.canonicalize().ok();
            let sibling_root = resolved.as_ref().and_then(|resolved| {
                siblings
                    .iter()
                    .find(|sibling| resolved.starts_with(&sibling.source))
            });
            let sibling = exists && sibling_root.is_some();
            let mapped_sibling = sibling_root.and_then(|sibling| {
                let resolved = resolved.as_ref()?;
                let within = resolved.strip_prefix(&sibling.source).ok()?;
                Some(
                    paths::relative_path(&cwd, &sibling.source.join(within))
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
            let exclusion = if written_words.contains(&token) {
                Some(ExclusionReason::CheckWritesPathInText)
            } else if manifest {
                Some(ExclusionReason::ManifestOrSetup)
            } else if sibling {
                Some(ExclusionReason::SiblingFromRoot)
            } else if runs {
                Some(ExclusionReason::CheckMechanics)
            } else if mapped.starts_with("../") {
                Some(ExclusionReason::OutsideCopiedTree)
            } else if !exists && !has_manifest_sibling {
                Some(ExclusionReason::MissingAtRoot)
            } else {
                None
            };
            (shaped || exists || has_manifest_sibling || written_words.contains(&token)).then_some(
                CandidateDiagnostic {
                    path: mapped,
                    exclusion,
                },
            )
        })
        .filter(|candidate| seen.insert(candidate.path.clone()))
        .collect();
    let eligible = candidates
        .iter()
        // #471 diagnostics report missing paths, but path-shaped inputs
        // remain eligible just as they were before the diagnostic pass.
        .filter(|candidate| {
            candidate.exclusion.is_none()
                || candidate.exclusion == Some(ExclusionReason::MissingAtRoot)
        })
        .map(|candidate| candidate.path.clone())
        .collect();
    InputDiscovery {
        eligible,
        candidates,
    }
}

/// `contract` with each of `inputs`' entries' executed text wrapped by
/// [`wrap`]. Nothing else changes, so each wrapped entry resolves exactly as
/// its original would.
pub(super) fn mutated(
    contract: &AcceptanceContract,
    inputs: &BTreeMap<String, Vec<String>>,
    live_roots: &[&Path],
    markers: &Markers,
) -> AcceptanceContract {
    let mut mutated = contract.clone();
    for entry in (mutated.acceptance.iter_mut()).chain(&mut mutated.supplementary) {
        let Some(names) = inputs.get(&entry.id) else {
            continue;
        };
        match &mut entry.check {
            AcceptanceCheck::Command { command, .. } => {
                *command = wrap(command, names, live_roots, markers);
            }
            AcceptanceCheck::Floor { contract } => {
                if let Some(command) = contract.typed_verifier_command.as_mut() {
                    *command = wrap(command, names, live_roots, markers);
                }
            }
        }
    }
    mutated
}

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
