//! The generic input mutation (A4): proving a check that passes on the
//! pre-implementation tree can still fail.
//!
//! A check can pass before any implementation for two reasons: it proves
//! nothing (it passes whatever the tree holds), or its criterion already
//! holds on that tree. Re-authoring cannot help the second, and a freeze
//! must not loop on it. So such a check is run once more, on the same tree,
//! with the DATA it names moved aside: every token of its executed text (and
//! a floor's `artifact_path`) that is a relative path, without `..`, that
//! exists under the check's working directory -- except what makes the
//! check run at all: a command word, the script an interpreter runs (`sh
//! x.sh`, `python3 x.py`), a directory it changes into (`cd`, `-C`), a
//! build manifest (`Cargo.toml`, `package.json`, `--manifest-path`, ...),
//! and any directory holding one of those. Moving those would make the
//! check fail without saying anything about its criterion.
//!
//! The mutation is a shell prologue and epilogue around the check's own
//! text, which runs in a subshell so its own `trap`/`exit` cannot skip the
//! restore. The prologue first sweeps what an earlier killed run left, then
//! moves each input aside unless a component of its path is a symlink
//! (following one could move a live file); the epilogue restores them in
//! REVERSE order (a nested input comes back before its parent) and only
//! then reports, on stdout, that everything was restored. Every marker
//! carries a per-run nonce, so a check cannot forge one. It is only ever
//! run in a hermetic copy, one per mutated check; the prologue refuses, with
//! a guard marker, to move anything when its working directory is inside a
//! live root.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, AcceptanceCriterion,
};

use super::executed_text;

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

/// The shell words of `text`, one list per simple command, and the bodies of
/// its here-documents (data, never commands). Quote-aware; a best effort
/// that errs towards excluding more.
fn lex(text: &str) -> (Vec<Vec<String>>, Vec<String>) {
    let (mut commands, mut bodies) = (vec![Vec::new()], Vec::new());
    let (mut word, mut quote, mut pending_tags): (String, Option<char>, Vec<String>) =
        (String::new(), None, Vec::new());
    let mut lines = text.lines();
    let finish = |word: &mut String, commands: &mut Vec<Vec<String>>| {
        if !word.is_empty() {
            commands
                .last_mut()
                .expect("one command")
                .push(std::mem::take(word));
        }
    };
    while let Some(line) = lines.next() {
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => word.push(c),
                None if c == '\'' || c == '"' => quote = Some(c),
                None if c == '<' && chars.get(i + 1) == Some(&'<') => {
                    finish(&mut word, &mut commands);
                    let rest: String = chars[i + 2..].iter().collect();
                    let tag: String = (rest.trim_start_matches(['-', ' ']).chars())
                        .filter(|c| !matches!(c, '\'' | '"'))
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !tag.is_empty() {
                        pending_tags.push(tag);
                    }
                    break;
                }
                None if c.is_whitespace() => finish(&mut word, &mut commands),
                None if matches!(c, ';' | '&' | '|' | '(' | ')' | '{' | '}' | '`') => {
                    finish(&mut word, &mut commands);
                    commands.push(Vec::new());
                }
                None if c == '$' && chars.get(i + 1) == Some(&'(') => {
                    finish(&mut word, &mut commands);
                    commands.push(Vec::new());
                    i += 1;
                }
                None => word.push(c),
            }
            i += 1;
        }
        if quote.is_some() {
            word.push('\n');
            continue;
        }
        finish(&mut word, &mut commands);
        commands.push(Vec::new());
        for tag in std::mem::take(&mut pending_tags) {
            let mut body = String::new();
            for line in lines.by_ref() {
                if line.trim() == tag {
                    break;
                }
                body.push_str(line);
                body.push('\n');
            }
            bodies.push(body);
        }
    }
    finish(&mut word, &mut commands);
    (commands, bodies)
}

/// The words of `text` that make the check run (see the module docs).
fn operational_words(commands: &[Vec<String>]) -> BTreeSet<String> {
    let mut excluded = BTreeSet::new();
    for words in commands {
        let mut rest = words
            .iter()
            .skip_while(|word| {
                PREFIXES.contains(&word.as_str())
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

fn normalize(token: &str) -> &str {
    token.trim_start_matches("./").trim_end_matches('/')
}

/// The relative path tokens of `text`.
fn path_tokens(text: &str) -> Vec<String> {
    let allowed =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-' | '+' | '@');
    text.split(|c: char| !allowed(c))
        .map(normalize)
        .filter(|token| {
            !token.is_empty()
                && !token.starts_with('-')
                && !token.starts_with('/')
                && token.len() <= 4096
                && Path::new(token).components().all(|part| {
                    matches!(part, std::path::Component::Normal(name) if name != ".git" && name != "target")
                })
        })
        .map(str::to_string)
        .collect()
}

/// Candidate data paths `entry` names, relative to its working directory.
pub(super) fn named_inputs(entry: &AcceptanceCriterion) -> Vec<String> {
    let text = executed_text(entry).map_or_else(String::new, |(_, text)| text.to_string());
    let (commands, bodies) = lex(&text);
    let excluded: Vec<String> = (operational_words(&commands).iter())
        .flat_map(|word| path_tokens(word))
        .collect();
    let mut words: Vec<String> = commands.into_iter().flatten().chain(bodies).collect();
    if let AcceptanceCheck::Floor { contract } = &entry.check {
        words.push(contract.artifact_path.clone());
    }
    let mut seen = BTreeSet::new();
    (words.iter())
        .flat_map(|word| path_tokens(word))
        .filter(|token| {
            let path = Path::new(token);
            let manifest = path
                .file_name()
                .is_some_and(|name| MANIFESTS.contains(&name.to_string_lossy().as_ref()));
            // Neither what runs the check nor a directory that holds it.
            let runs = (excluded.iter()).any(|word| Path::new(word).starts_with(path));
            !manifest && !runs
        })
        .filter(|token| seen.insert(token.clone()))
        .collect()
}

/// The markers of one mutated run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Markers {
    nonce: String,
}

impl Markers {
    pub(super) fn new() -> Self {
        Self {
            nonce: uuid::Uuid::new_v4().simple().to_string(),
        }
    }

    fn tag(&self) -> String {
        format!("archon-mutated-{}", &self.nonce[..12])
    }

    fn moved_prefix(&self) -> String {
        format!("archon-mutation-moved-{}: ", self.nonce)
    }

    fn restored(&self) -> String {
        format!("archon-mutation-restored-{}", self.nonce)
    }

    fn guard(&self) -> String {
        format!(
            "archon-mutation-guard-{}: refusing to move inputs in a live root",
            self.nonce
        )
    }

    /// The inputs a run moved aside, as its prologue reported them.
    pub(super) fn moved(&self, result: &CheckResult) -> Vec<String> {
        let prefix = self.moved_prefix();
        String::from_utf8_lossy(&result.stderr)
            .lines()
            .filter_map(|line| line.strip_prefix(prefix.as_str()))
            .map(str::to_string)
            .collect()
    }

    /// Whether the run restored every input and left nothing behind.
    pub(super) fn restored_all(&self, result: &CheckResult) -> bool {
        let restored = self.restored();
        String::from_utf8_lossy(&result.stdout)
            .lines()
            .any(|line| line == restored)
    }

    /// Whether the run stopped at the live-root guard.
    pub(super) fn guarded(&self, result: &CheckResult) -> bool {
        result.exit_code == Some(GUARD_STATUS)
            && String::from_utf8_lossy(&result.stderr).contains(&self.guard())
    }
}

/// The check text wrapped so `inputs` are moved aside, the check runs, and
/// they are restored; its exit status is the check's.
pub(super) fn wrap(
    original: &str,
    inputs: &[String],
    live_roots: &[&Path],
    markers: &Markers,
) -> String {
    let quote = |text: &str| format!("'{}'", text.replace('\'', r"'\''"));
    let tag = markers.tag();
    let mut script = String::new();
    for root in live_roots {
        let root = quote(&root.to_string_lossy());
        script.push_str(&format!(
            "case \"$(pwd -P)/\" in {root}/*) printf '%s\\n' {} >&2; exit {GUARD_STATUS};; esac\n",
            quote(&markers.guard())
        ));
    }
    let sweep =
        "find . \\( -name .git -o -name target \\) -prune -o -name '*.archon-mutated-*' -print";
    script.push_str(&format!(
        "{sweep} | while IFS= read -r __archon_left; do __archon_base=\"${{__archon_left%.archon-mutated-*}}\"; if [ -e \"$__archon_base\" ] || [ -L \"$__archon_base\" ]; then rm -rf -- \"$__archon_left\"; else mv -- \"$__archon_left\" \"$__archon_base\"; fi; done\n"
    ));
    for input in inputs {
        let mut prefix = String::new();
        let mut plain = Vec::new();
        for part in input.split('/') {
            prefix = if prefix.is_empty() {
                part.to_string()
            } else {
                format!("{prefix}/{part}")
            };
            plain.push(format!("[ ! -L {} ]", quote(&prefix)));
        }
        let path = quote(input);
        script.push_str(&format!(
            "if {} && [ -e {path} ]; then mv -- {path} {path}.{tag} 2>/dev/null && printf '%s%s\\n' {} {path} >&2; fi\n",
            plain.join(" && "),
            quote(&markers.moved_prefix()),
        ));
    }
    script.push_str(&format!("(\n{original}\n)\n__archon_status=$?\n"));
    for input in inputs.iter().rev() {
        let path = quote(input);
        script.push_str(&format!(
            "if [ -e {path}.{tag} ] || [ -L {path}.{tag} ]; then rm -rf -- {path}; mv -- {path}.{tag} {path}; fi\n"
        ));
    }
    script.push_str(&format!(
        "if [ -z \"$({sweep} | grep -F -- {} | head -n 1)\" ]; then printf '%s\\n' {}; fi\nexit $__archon_status\n",
        quote(&tag),
        quote(&markers.restored()),
    ));
    script
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
