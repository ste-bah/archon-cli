//! Obs-123: which directories a shell command searches RECURSIVELY.
//!
//! Live, an acceptance-remediation coder ran `grep -rn ... <project root>`:
//! the project root holds the run store, so the search walked every run's
//! worktrees and records on the machine for over half an hour. The built-in
//! Grep/Glob tools already prune the store (`RunStoreScope::prunes_walk`);
//! a shell search had nothing. This reads the roots a recursive search
//! names, so the run-store scope can refuse one that would walk the store or
//! leave the call's workspace.
//!
//! Recognised, after compound-statement syntax (`(`, `{`, `if ...; then`)
//! and the process wrappers (`unwrapped`): `grep`/`egrep`/`fgrep` with a
//! recursive flag (`-r`/`-R` in any short cluster, `--recursive`,
//! `--dereference-recursive`, `-d recurse`, `--directories=recurse`); `rg`,
//! `ag` and `ack`, which recurse by default; `find`; `ls` with
//! `-R`/`--recursive`; and `tree`. A search naming no root searches the
//! working directory. Value-taking options are skipped with their value,
//! attached (`-efoo`, `--regexp=foo`) or not; the first operand of a
//! grep-like search is its pattern unless a pattern option was given (or
//! `--files`). `rg`/`ag`, `tree` and `ls` skip hidden directories unless told
//! otherwise, which the caller weighs. `~` and `$HOME` are expanded; a root
//! behind any other expansion, or after a `cd` to an unknown directory, is
//! not placed. Not covered: a search run through `xargs`, `sh -c` or a
//! script; like the rest of the guard this is an efficiency rule, not a
//! sandbox.

use super::{commands, statement_words, unwrapped};

/// One recursive search: the program, each root it names (anchored to the
/// directory a preceding `cd`/`pushd` moved to; `None` when that directory,
/// or the root itself, cannot be placed, e.g. behind a variable), and
/// whether it skips hidden directories (`rg`/`ag`, `tree`, `ls -R` without
/// their show-hidden flags), which decides whether a store under a hidden
/// directory is walked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workflow_read_guard) struct RecursiveSearch {
    pub program: String,
    pub roots: Vec<Option<String>>,
    pub skips_hidden: bool,
}

pub(in crate::workflow_read_guard) fn recursive_searches(command: &str) -> Vec<RecursiveSearch> {
    let mut cwd: Option<Option<String>> = None; // None: the working root.
    let mut found = Vec::new();
    for words in commands(command) {
        let words = statement_words(&words);
        let words = unwrapped(&words);
        let Some((program, args)) = words.split_first() else {
            continue;
        };
        let name = program.rsplit('/').next().unwrap_or(program);
        match name {
            "cd" | "pushd" => {
                let target = args.iter().find(|a| !a.starts_with('-'));
                cwd = Some(target.and_then(|dir| anchor(cwd.as_ref(), dir)));
                continue;
            }
            "popd" => {
                cwd = Some(None);
                continue;
            }
            _ => {}
        }
        let Some(search) = search_roots(name, args) else {
            continue;
        };
        let roots = if search.roots.is_empty() {
            vec![".".to_string()]
        } else {
            search.roots
        };
        found.push(RecursiveSearch {
            program: name.to_string(),
            roots: roots
                .iter()
                .map(|root| anchor(cwd.as_ref(), root))
                .collect(),
            skips_hidden: search.skips_hidden,
        });
    }
    found
}

/// `root` placed: `~` and `$HOME` expanded, absolute as it is, relative
/// joined to a known directory; `None` under an unknown directory or behind
/// any other expansion.
fn anchor(cwd: Option<&Option<String>>, root: &str) -> Option<String> {
    let home = || std::env::var("HOME").ok().filter(|h| !h.is_empty());
    let root = if root == "~" || root == "$HOME" || root == "${HOME}" {
        home()?
    } else if let Some(rest) = ["~/", "$HOME/", "${HOME}/"]
        .iter()
        .find_map(|prefix| root.strip_prefix(prefix))
    {
        format!("{}/{rest}", home()?)
    } else {
        root.to_string()
    };
    if root.contains(['$', '`']) || root.starts_with('~') {
        return None;
    }
    if archon_write_plan::lexical_path::rooted(&root) {
        return Some(root);
    }
    match cwd {
        None => Some(root),
        Some(Some(dir)) => Some(format!("{}/{root}", dir.trim_end_matches('/'))),
        Some(None) => None,
    }
}

/// What one command's words say about its search.
struct Parsed {
    roots: Vec<String>,
    skips_hidden: bool,
}

/// The roots a recursive search by `name` names (possibly none: the working
/// directory), or `None` when the command is not a recursive search.
fn search_roots(name: &str, args: &[String]) -> Option<Parsed> {
    let parsed = |spec: &Spec| parse(args, spec);
    match name {
        "grep" | "egrep" | "fgrep" => {
            let p = parsed(&GREP);
            let recursive = p.flags.iter().any(|f| matches!(f.as_str(), "r" | "R"))
                || p.longs.iter().any(|l| {
                    matches!(
                        l.as_str(),
                        "--recursive" | "--dereference-recursive" | "--directories=recurse"
                    )
                })
                || p.values.iter().any(|(opt, v)| opt == "d" && v == "recurse");
            recursive.then(|| Parsed {
                roots: p.operands_past_pattern(true),
                skips_hidden: false,
            })
        }
        "rg" | "ag" => {
            let p = parsed(&RG);
            let files = p.longs.iter().any(|l| l == "--files");
            let hidden = p.flags.iter().any(|f| matches!(f.as_str(), "u" | "."))
                || p.longs.iter().any(|l| {
                    l == "--hidden" || l == "--unrestricted" || l.starts_with("--no-ignore")
                });
            Some(Parsed {
                roots: p.operands_past_pattern(!files),
                skips_hidden: !hidden,
            })
        }
        "ack" => Some(Parsed {
            roots: parsed(&RG).operands_past_pattern(true),
            skips_hidden: false,
        }),
        "find" => Some(Parsed {
            roots: args
                .iter()
                .skip_while(|a| matches!(a.as_str(), "-H" | "-L" | "-P"))
                .take_while(|a| !a.starts_with(['-', '(', '!', ')']))
                .cloned()
                .collect(),
            skips_hidden: false,
        }),
        "ls" => {
            let p = parsed(&LS);
            let recursive =
                p.flags.iter().any(|f| f == "R") || p.longs.iter().any(|l| l == "--recursive");
            let hidden = p.flags.iter().any(|f| matches!(f.as_str(), "a" | "A"))
                || p.longs.iter().any(|l| l == "--all" || l == "--almost-all");
            recursive.then(|| Parsed {
                roots: p.operands.clone(),
                skips_hidden: !hidden,
            })
        }
        "tree" => {
            let p = parsed(&TREE);
            Some(Parsed {
                skips_hidden: !p.flags.iter().any(|f| f == "a"),
                roots: p.operands,
            })
        }
        _ => None,
    }
}

/// Which options of a program take a value: single-letter short options
/// (the value is the rest of the cluster, or the next word) and long
/// options (the next word, unless written `--opt=value`).
struct Spec {
    short_valued: &'static str,
    long_valued: &'static [&'static str],
}

const GREP: Spec = Spec {
    short_valued: "efmABCdD",
    long_valued: &[
        "--regexp",
        "--file",
        "--max-count",
        "--after-context",
        "--before-context",
        "--context",
        "--include",
        "--exclude",
        "--exclude-dir",
        "--exclude-from",
        "--directories",
        "--devices",
        "--label",
    ],
};
const RG: Spec = Spec {
    short_valued: "efgtTmABCjMEr",
    long_valued: &[
        "--regexp",
        "--file",
        "--glob",
        "--iglob",
        "--type",
        "--type-not",
        "--max-count",
        "--after-context",
        "--before-context",
        "--context",
        "--threads",
        "--max-columns",
        "--encoding",
        "--replace",
        "--max-depth",
        "--max-filesize",
        "--ignore-file",
        "--type-add",
        "--sort",
        "--sortr",
        "--pre",
        "--pre-glob",
        "--path-separator",
        "--colors",
        "--color",
        "--depth",
        "--ignore-dir",
        "--file-search-regex",
    ],
};
const LS: Spec = Spec {
    short_valued: "ITw",
    long_valued: &[
        "--block-size",
        "--format",
        "--hide",
        "--ignore",
        "--sort",
        "--width",
    ],
};
const TREE: Spec = Spec {
    short_valued: "LIPo",
    long_valued: &["--filelimit", "--timefmt"],
};

#[derive(Default)]
struct Options {
    /// Short flags seen, one letter each, value-taking ones included.
    flags: Vec<String>,
    /// Long options as written (with any `=value`).
    longs: Vec<String>,
    /// Short value-taking options and their values.
    values: Vec<(String, String)>,
    operands: Vec<String>,
    /// A pattern option (`-e`, `-f`, `--regexp`, `--file`) was given.
    pattern_given: bool,
}

impl Options {
    fn operands_past_pattern(&self, pattern_first: bool) -> Vec<String> {
        let mut out = self.operands.clone();
        if pattern_first && !self.pattern_given && !out.is_empty() {
            out.remove(0);
        }
        out
    }
}

fn parse(args: &[String], spec: &Spec) -> Options {
    let mut out = Options::default();
    let mut words = args.iter();
    let mut options_done = false;
    while let Some(word) = words.next() {
        if options_done || word == "-" || !word.starts_with('-') {
            out.operands.push(word.clone());
            continue;
        }
        if word == "--" {
            options_done = true;
            continue;
        }
        if let Some(long) = word.strip_prefix("--") {
            let name = format!("--{}", long.split('=').next().unwrap_or(long));
            if matches!(name.as_str(), "--regexp" | "--file") {
                out.pattern_given = true;
            }
            if !long.contains('=') && spec.long_valued.contains(&name.as_str()) {
                words.next();
            }
            out.longs.push(word.clone());
            continue;
        }
        let cluster: Vec<char> = word[1..].chars().collect();
        for (at, letter) in cluster.iter().enumerate() {
            out.flags.push(letter.to_string());
            if !spec.short_valued.contains(*letter) {
                continue;
            }
            if matches!(letter, 'e' | 'f') {
                out.pattern_given = true;
            }
            let inline: String = cluster[at + 1..].iter().collect();
            let value = if inline.is_empty() {
                words.next().cloned().unwrap_or_default()
            } else {
                inline
            };
            out.values.push((letter.to_string(), value));
            break;
        }
    }
    out
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn short_names_and_drive_roots_remain_known_after_directory_changes() {
        let searches =
            recursive_searches("cd 'C:/elsewhere' && grep -r needle 'C:/Users/RUNNER~1/project'");
        assert_eq!(
            searches[0].roots,
            vec![Some("C:/Users/RUNNER~1/project".into())]
        );
        let searches = recursive_searches("cd $DIR && grep -r needle 'C:/project'");
        assert_eq!(searches[0].roots, vec![Some("C:/project".into())]);
        assert_eq!(anchor(None, "~unknown/project"), None);
        assert_eq!(anchor(None, "$UNKNOWN/project"), None);
    }
}
