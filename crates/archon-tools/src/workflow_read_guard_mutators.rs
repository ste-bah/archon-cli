//! Tree-wide mutators: formatters and fixers that rewrite every file in the
//! tree unless told a scope. A write agent's patch is judged against its
//! declared `target_files`, so every file such a run touches outside them is
//! an undeclared change; refusing the unscoped form up front costs the agent
//! one tool call instead of costing the wave its branch. Live on wf-7db01ce7
//! `agents-3-0`: one `cargo fmt --all`, sixty-four files, the real work
//! stranded (Issue-13).
//!
//! Data-driven and language-agnostic: a rule is a program, an optional
//! subcommand, the flags that make it write, the flags that make it read-only,
//! the flags that scope it, and the scoped form to suggest. Operands are read
//! syntactically — an explicit file has an extension; `.`, a directory, a
//! glob or nothing at all is the whole tree. This is a workflow efficiency
//! guard, not a shell sandbox: a shape it does not recognise runs.

use serde::{Deserialize, Serialize};

/// One command shape that rewrites the whole tree unless given a scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TreeWideMutator {
    /// Program in the executable position, basename only (`cargo`, `prettier`).
    pub program: String,
    /// Subcommand that must follow the program (`fmt` for `cargo fmt`); empty for none.
    pub subcommand: String,
    /// The command writes only when one of these is present (`-w` for gofmt,
    /// `--write` for prettier, `--fix` for eslint). Empty: it always writes.
    pub mutating_flags: Vec<String>,
    /// Flags that make it read-only (`--check`, `--diff`); any present: not a mutation.
    pub dry_run_flags: Vec<String>,
    /// Flags that name an explicit scope (`-p`/`--package` for cargo fmt,
    /// `--include` for dotnet format); any present: scoped, allowed.
    pub scope_flags: Vec<String>,
    /// Flags that consume the next argument, so a config path is never read as
    /// a file operand (`--config x`, `--manifest-path p`).
    pub value_flags: Vec<String>,
    /// Whether an explicit file operand (`src/a.ts`) scopes the command. False
    /// for `dotnet format`, whose operand is a project or solution, not a file
    /// to format.
    pub file_operands_scope: bool,
    /// The scoped form the refusal suggests, in the agent's own terms.
    pub scoped_form: String,
}

impl Default for TreeWideMutator {
    fn default() -> Self {
        Self {
            program: String::new(),
            subcommand: String::new(),
            mutating_flags: Vec::new(),
            dry_run_flags: Vec::new(),
            scope_flags: Vec::new(),
            value_flags: Vec::new(),
            file_operands_scope: true,
            scoped_form: String::new(),
        }
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

fn rule(program: &str, subcommand: &str, scoped_form: &str) -> TreeWideMutator {
    TreeWideMutator {
        program: program.into(),
        subcommand: subcommand.into(),
        scoped_form: scoped_form.into(),
        ..TreeWideMutator::default()
    }
}

/// The built-in rules, used when `workflow.generated.tree_wide_mutators` is unset.
pub fn default_tree_wide_mutators() -> Vec<TreeWideMutator> {
    vec![
        TreeWideMutator {
            dry_run_flags: strings(&["--check"]),
            scope_flags: strings(&["-p", "--package"]),
            value_flags: strings(&[
                "--manifest-path",
                "--message-format",
                "--config",
                "--config-path",
                "--edition",
                "--color",
            ]),
            ..rule(
                "cargo",
                "fmt",
                "cargo fmt -p <crate> -- <file>, or rustfmt <file>",
            )
        },
        TreeWideMutator {
            dry_run_flags: strings(&["--check"]),
            value_flags: strings(&[
                "--config",
                "--config-path",
                "--edition",
                "--color",
                "--print-config",
                "--emit",
            ]),
            ..rule("rustfmt", "", "rustfmt <file>")
        },
        TreeWideMutator {
            mutating_flags: strings(&["-w"]),
            value_flags: strings(&["-r", "-cpuprofile"]),
            ..rule("gofmt", "", "gofmt -w <file>")
        },
        TreeWideMutator {
            mutating_flags: strings(&["-w"]),
            value_flags: strings(&["-local", "-srcdir", "-cpuprofile"]),
            ..rule("goimports", "", "goimports -w <file>")
        },
        TreeWideMutator {
            mutating_flags: strings(&["-w"]),
            ..rule("gofumpt", "", "gofumpt -w <file>")
        },
        TreeWideMutator {
            dry_run_flags: strings(&["--check", "--diff"]),
            value_flags: strings(&[
                "--config",
                "-l",
                "--line-length",
                "-t",
                "--target-version",
                "--include",
                "--exclude",
                "--extend-exclude",
                "--force-exclude",
                "--stdin-filename",
                "-W",
                "--workers",
                "--required-version",
            ]),
            ..rule("black", "", "black <file>")
        },
        TreeWideMutator {
            dry_run_flags: strings(&["--check", "--check-only", "-c", "--diff", "-d"]),
            value_flags: strings(&[
                "--settings-path",
                "--settings-file",
                "--settings",
                "--sp",
                "--cr",
                "--resolve-all-configs",
                "-l",
                "--line-length",
                "-s",
                "--skip",
                "--extend-skip",
                "-p",
                "--project",
                "-o",
                "--thirdparty",
                "--profile",
                "-j",
                "--jobs",
            ]),
            ..rule("isort", "", "isort <file>")
        },
        TreeWideMutator {
            dry_run_flags: strings(&["--check", "--diff"]),
            value_flags: strings(&[
                "--config",
                "--line-length",
                "--target-version",
                "--exclude",
                "--extend-exclude",
                "--stdin-filename",
                "--range",
            ]),
            ..rule("ruff", "format", "ruff format <file>")
        },
        TreeWideMutator {
            mutating_flags: strings(&["--fix", "--fix-only"]),
            dry_run_flags: strings(&["--diff"]),
            value_flags: strings(&[
                "--config",
                "--select",
                "--extend-select",
                "--ignore",
                "--extend-ignore",
                "--exclude",
                "--extend-exclude",
                "--line-length",
                "--target-version",
                "--output-format",
                "-o",
                "--output-file",
                "--stdin-filename",
                "--fixable",
                "--unfixable",
            ]),
            ..rule("ruff", "check", "ruff check --fix <file>")
        },
        TreeWideMutator {
            mutating_flags: strings(&["--write", "-w"]),
            value_flags: strings(&[
                "--config",
                "--ignore-path",
                "--log-level",
                "--parser",
                "--plugin",
                "--cache-location",
                "--cache-strategy",
                "--stdin-filepath",
                "--config-precedence",
            ]),
            ..rule("prettier", "", "prettier --write <file>")
        },
        TreeWideMutator {
            mutating_flags: strings(&["--fix"]),
            dry_run_flags: strings(&["--fix-dry-run"]),
            value_flags: strings(&[
                "-c",
                "--config",
                "--ext",
                "--ignore-path",
                "--ignore-pattern",
                "--rule",
                "--rulesdir",
                "--parser",
                "--parser-options",
                "--env",
                "--global",
                "--plugin",
                "-f",
                "--format",
                "-o",
                "--output-file",
                "--max-warnings",
                "--resolve-plugins-relative-to",
                "--cache-location",
                "--cache-strategy",
                "--stdin-filename",
            ]),
            ..rule("eslint", "", "eslint --fix <file>")
        },
        TreeWideMutator {
            dry_run_flags: strings(&["--verify-no-changes"]),
            scope_flags: strings(&["--include"]),
            value_flags: strings(&[
                "--exclude",
                "-v",
                "--verbosity",
                "--severity",
                "--diagnostics",
                "--binarylog",
                "--report",
            ]),
            file_operands_scope: false,
            ..rule(
                "dotnet",
                "format",
                "dotnet format <project> --include <file>",
            )
        },
    ]
}

/// The refusal for `command` when one of its executable segments is a
/// tree-wide mutator without an explicit scope; `None` when every segment is
/// scoped, read-only, or unrecognised.
pub(super) fn tree_wide_mutation(
    segments: &[Vec<String>],
    rules: &[TreeWideMutator],
) -> Option<String> {
    segments.iter().find_map(|words| {
        let (name, args) = super::shell::program(words);
        let (name, args) = unwrap_runner(name, args);
        let rule = rules
            .iter()
            .find(|rule| rule.program == name && unscoped_mutation(rule, args))?;
        let head = super::clip(&words.join(" "), super::RECORD_HEAD_CHARS);
        Some(format!(
            "`{head}` is refused: it rewrites the whole tree, and every file it touches outside \
             your declared target_files is an undeclared change that is dropped from your patch. \
             Run the formatter on the files you changed: {}. The operator may enable \
             workflow.generated.allow_tree_wide_mutators.",
            rule.scoped_form
        ))
    })
}

/// Strip a runner prefix so `npx prettier`, `pnpm exec eslint`, `poetry run
/// black` and `python -m black` are judged as the program they run.
fn unwrap_runner<'a>(name: &'a str, args: &'a [String]) -> (&'a str, &'a [String]) {
    let shift = match name {
        "npx" | "bunx" | "uvx" => args.iter().position(|a| !a.starts_with('-')).map(|i| i + 1),
        "pnpm" | "yarn" | "poetry" | "uv" | "pipenv"
            if args
                .first()
                .is_some_and(|a| matches!(a.as_str(), "exec" | "dlx" | "run")) =>
        {
            Some(2)
        }
        "python" | "python3" if args.first().is_some_and(|a| a == "-m") => Some(2),
        _ => None,
    };
    match shift {
        Some(shift) if shift <= args.len() && shift > 0 => {
            let program = &args[shift - 1];
            (
                program.rsplit('/').next().unwrap_or(program),
                &args[shift..],
            )
        }
        _ => (name, args),
    }
}

fn flag_present(flags: &[String], arg: &str) -> bool {
    flags
        .iter()
        .any(|flag| arg == flag || (flag.starts_with("--") && arg.starts_with(&format!("{flag}="))))
}

/// Whether `args` (after the program) match `rule` as a write over the whole
/// tree: the subcommand is there, no dry-run flag, a mutating flag when one is
/// required, no scope flag, and no explicit file operand.
fn unscoped_mutation(rule: &TreeWideMutator, args: &[String]) -> bool {
    let args = if args.first().is_some_and(|a| a.starts_with('+')) {
        &args[1..]
    } else {
        args
    };
    let rest = if rule.subcommand.is_empty() {
        args
    } else {
        match args.iter().position(|a| !a.starts_with('-')) {
            Some(i) if args[i] == rule.subcommand => &args[i + 1..],
            _ => return false,
        }
    };
    let mut writes = rule.mutating_flags.is_empty();
    let mut operands: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i].as_str();
        // Past `--` the tool's own flags follow (`cargo fmt -- --check`):
        // a dry-run flag there still means read-only, other flags are inert.
        if arg == "--" {
            for arg in rest[i + 1..].iter().map(String::as_str) {
                if flag_present(&rule.dry_run_flags, arg) {
                    return false;
                }
                if !arg.starts_with('-') || arg == "-" {
                    operands.push(arg);
                }
            }
            break;
        }
        if flag_present(&rule.dry_run_flags, arg) {
            return false;
        }
        if flag_present(&rule.scope_flags, arg) {
            return false;
        }
        if flag_present(&rule.mutating_flags, arg) {
            writes = true;
        }
        if arg.starts_with('-') && arg != "-" {
            if rule.value_flags.iter().any(|flag| flag == arg) {
                i += 1;
            }
        } else {
            operands.push(arg);
        }
        i += 1;
    }
    if !writes {
        return false;
    }
    let explicitly_scoped = rule.file_operands_scope
        && !operands.is_empty()
        && operands.iter().all(|operand| file_operand(operand));
    !explicitly_scoped
}

/// An operand that names one file: a last path segment with an extension, no
/// glob characters. `.`, `./`, a directory, `./...` or `src/**/*.ts` are not.
fn file_operand(operand: &str) -> bool {
    if operand == "-" || operand.ends_with('/') || operand.contains(['*', '?', '[']) {
        return false;
    }
    let name = operand.rsplit('/').next().unwrap_or(operand);
    if name == "." || name == ".." || name == "..." {
        return false;
    }
    name.rfind('.')
        .is_some_and(|dot| dot > 0 && dot + 1 < name.len())
}
