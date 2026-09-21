//! The file a shell command will write, when its target can be read off the
//! command text: `sed -i … <file>`, a stdout/stderr redirection (`cat >
//! file`, `echo x >> file`), `tee [-a] <file>`, and a Python `open('<file>',
//! 'w')` literal. Everything else — a script run from a file, a variable in
//! the path, `cp`/`mv` whose destination may be a directory, a command that
//! `cd`s first and then names a relative path — is not recoverable
//! syntactically and is not judged. This is a workflow efficiency guard, not
//! a shell sandbox: a shape it cannot parse runs.

use super::shell;

/// One write a command announces: the segment it appears in (for the
/// refusal) and the path exactly as the command spells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShellWrite {
    pub(super) head: String,
    pub(super) path: String,
}

/// Every syntactically recoverable write target in `command`, in order.
pub(super) fn write_targets(command: &str) -> Vec<ShellWrite> {
    let mut found: Vec<ShellWrite> = Vec::new();
    // After a `cd`/`pushd` the working directory is unknown, so a later
    // relative path can no longer be resolved against the worktree root.
    let mut cwd_moved = false;
    let segments = shell::commands(command);
    let mut python_seen = false;
    for words in &segments {
        let head = super::clip(&words.join(" "), super::RECORD_HEAD_CHARS);
        let mut push = |path: &str| {
            if literal_path(path) && (!cwd_moved || path.starts_with('/')) {
                found.push(ShellWrite {
                    head: head.clone(),
                    path: path.to_string(),
                });
            }
        };
        for (index, word) in words.iter().enumerate() {
            if is_output_redirect(word)
                && let Some(destination) = words.get(index + 1)
            {
                push(destination);
            }
        }
        let (name, args) = shell::program(words);
        match name {
            "cd" | "pushd" => cwd_moved = true,
            "sed" => {
                for file in sed_in_place_files(args) {
                    push(file);
                }
            }
            "tee" => {
                for file in tee_files(args) {
                    push(file);
                }
            }
            "python" | "python3" | "py" => python_seen = true,
            _ => {}
        }
    }
    if python_seen {
        let mut push = |path: &str| {
            if literal_path(path) && (!cwd_moved || path.starts_with('/')) {
                found.push(ShellWrite {
                    head: super::clip(&super::normalise_command(command), super::RECORD_HEAD_CHARS),
                    path: path.to_string(),
                });
            }
        };
        for path in python_open_writes(command) {
            push(&path);
        }
    }
    found
}

/// A redirection word the lexer produced that sends a stream to a FILE:
/// `>`, `>>`, `1>`, `2>`, `2>>`; not `>&`/`2>&1` (a descriptor) and not `<`.
fn is_output_redirect(word: &str) -> bool {
    let stripped = word.trim_start_matches(|c: char| c.is_ascii_digit());
    matches!(stripped, ">" | ">>" | ">|")
}

/// A path that is spelled out: no expansion, no glob, no descriptor.
fn literal_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('&')
        && !path.starts_with('(')
        && !path.contains(['$', '`', '*', '?', '{', '~'])
}

/// `args` with every redirection operator and its operand removed: the
/// `2>/dev/null` or `< in` after a program's files is not one of its files.
fn without_redirects(args: &[String]) -> Vec<&str> {
    let mut kept = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if shell::redirect_operator(&args[i]) {
            i += 2;
        } else {
            kept.push(args[i].as_str());
            i += 1;
        }
    }
    kept
}

/// The file operands of a `sed` run in place. `None`-shaped otherwise: an
/// `-i`-less sed reads. The script is the `-e`/`-f` value, or the first
/// operand when neither is given; everything after it is a file.
fn sed_in_place_files(args: &[String]) -> Vec<&str> {
    let args = without_redirects(args);
    let mut in_place = false;
    let mut script_given = false;
    let mut operands: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i];
        match arg {
            "-e" | "--expression" | "-f" | "--file" => {
                script_given = true;
                i += 2;
                continue;
            }
            "--" => {
                operands.extend(&args[i + 1..]);
                break;
            }
            _ => {}
        }
        if arg.starts_with("--expression=") || arg.starts_with("--file=") {
            script_given = true;
        } else if arg.starts_with("--in-place") {
            in_place = true;
        } else if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 1 {
            // A short cluster: `-i`, `-i.bak`, `-ni`, `-Ei`. An `e`/`f` inside
            // the cluster consumes the next word as the script.
            let cluster = &arg[1..];
            if cluster.contains('i') {
                in_place = true;
            }
            if cluster.contains(['e', 'f']) && !cluster.starts_with('i') {
                script_given = true;
                i += 1;
            }
        } else if !arg.starts_with('-') {
            operands.push(arg);
        }
        i += 1;
    }
    if !in_place {
        return Vec::new();
    }
    if !script_given && !operands.is_empty() {
        operands.remove(0);
    }
    operands
}

/// The file operands of `tee`; its flags take no value except
/// `--output-error[=MODE]`, which is inline.
fn tee_files(args: &[String]) -> Vec<&str> {
    without_redirects(args)
        .into_iter()
        .filter(|arg| !arg.starts_with('-'))
        .collect()
}

/// Every `open('<path>', '<mode>')` / `open("<path>", mode="<mode>")` in the
/// text whose mode writes (`w`, `a`, `x`, `+`). Both arguments must be
/// string literals; a variable, an f-string or a `Path(...)` is not read.
fn python_open_writes(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("open(") {
        rest = &rest[at + "open(".len()..];
        let Some((path, after)) = string_literal(rest.trim_start()) else {
            continue;
        };
        let after = after.trim_start();
        let Some(after) = after.strip_prefix(',') else {
            continue;
        };
        let after = after.trim_start();
        let after = after.strip_prefix("mode").map_or(after, |a| {
            a.trim_start().strip_prefix('=').unwrap_or(a).trim_start()
        });
        let Some((mode, _)) = string_literal(after) else {
            continue;
        };
        if mode.contains(['w', 'a', 'x', '+']) && !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

/// A single- or double-quoted literal at the start of `text`, and the text
/// after its closing quote. No escapes: a path with a quote in it is not a
/// path this reads.
fn string_literal(text: &str) -> Option<(String, &str)> {
    let quote = text.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let body = &text[1..];
    let end = body.find(quote)?;
    Some((body[..end].to_string(), &body[end + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(command: &str) -> Vec<String> {
        write_targets(command).into_iter().map(|w| w.path).collect()
    }

    #[test]
    fn redirections_and_tee_name_their_files_and_descriptors_do_not() {
        assert_eq!(paths("cat > src/new.rs"), vec!["src/new.rs"]);
        assert_eq!(paths("echo x >> notes/log.md"), vec!["notes/log.md"]);
        assert_eq!(
            paths("cargo test 2>&1 | tee -a target/out.txt"),
            vec!["target/out.txt"]
        );
        assert_eq!(paths("printf 'x' 2> /dev/null"), vec!["/dev/null"]);
        assert!(paths("cargo test 2>&1 >&2").is_empty());
        assert!(paths("cat < src/lib.rs").is_empty());
        assert!(paths("echo x > $OUT").is_empty());
    }

    #[test]
    fn sed_in_place_names_the_files_after_the_script_and_a_plain_sed_reads() {
        assert_eq!(paths("sed -i 's/a/b/' src/a.rs"), vec!["src/a.rs"]);
        assert_eq!(
            paths("sed -i '' -e 's/a/b/' src/a.rs src/b.rs"),
            vec!["src/a.rs", "src/b.rs"]
        );
        assert_eq!(paths("sed -i.bak 's/a/b/' src/a.rs"), vec!["src/a.rs"]);
        assert_eq!(
            paths("sed --in-place=.bak 's/a/b/' src/a.rs"),
            vec!["src/a.rs"]
        );
        assert_eq!(paths("sed -ni 's/a/b/p' src/a.rs"), vec!["src/a.rs"]);
        assert!(paths("sed -n 's/a/b/p' src/a.rs").is_empty());
        assert!(paths("sed 's/a/b/' src/a.rs > /tmp/x").len() == 1);
    }

    #[test]
    fn python_open_literals_with_a_writing_mode_are_read_and_others_are_not() {
        assert_eq!(
            paths("python3 -c \"open('src/gen.rs','w').write('x')\""),
            vec!["src/gen.rs"]
        );
        assert_eq!(
            paths(
                "python3 - <<'EOF'\nwith open(\"docs/out.md\", mode=\"a\") as f:\n    f.write('x')\nEOF"
            ),
            vec!["docs/out.md"]
        );
        assert!(paths("python3 -c \"open('src/a.rs').read()\"").is_empty());
        assert!(paths("python3 -c \"open('src/a.rs', 'r').read()\"").is_empty());
        assert!(paths("python3 -c \"open(path, 'w')\"").is_empty());
        assert!(paths("echo \"open('src/a.rs','w')\"").is_empty());
    }

    #[test]
    fn a_heredoc_body_is_data_and_only_the_outer_redirect_is_a_write() {
        assert_eq!(
            paths(
                "cat >> /abs/path/log.md <<'EOF'\nsome prose\nx = > The honest answer is\n\
                 2> /abs/err\ncargo test | tee /abs/x\nsed -i 's/a/b/' /abs/y\nrm -rf /\ncd /\nEOF"
            ),
            vec!["/abs/path/log.md"]
        );
        assert!(paths("cat <<EOF\n> /abs/x\nEOF").is_empty());
        // The line after the terminator is executable again.
        assert_eq!(
            paths("cat <<EOF\n> /abs/x\nEOF\necho y > /abs/z"),
            vec!["/abs/z"]
        );
    }

    #[test]
    fn sed_and_tee_skip_a_redirect_operator_and_its_operand() {
        assert_eq!(
            paths("sed -i '' 's/a/b/' /abs/file 2>&1"),
            vec!["/abs/file"]
        );
        assert_eq!(paths("tee /abs/out 2>&1"), vec!["/abs/out"]);
        assert_eq!(paths("tee -a /abs/out < /abs/in"), vec!["/abs/out"]);
        // The stderr redirect itself is still reported by the redirect scan;
        // `2>` and `/dev/null` are not sed's or tee's operands.
        assert_eq!(
            paths("sed -i '' 's/a/b/' \"/abs/file\" 2>/dev/null"),
            vec!["/dev/null", "/abs/file"]
        );
        assert_eq!(
            paths("tee /abs/out 2>/dev/null"),
            vec!["/dev/null", "/abs/out"]
        );
    }

    #[test]
    fn a_relative_path_after_cd_is_not_judged_but_an_absolute_one_is() {
        assert!(paths("cd crates/x && sed -i 's/a/b/' src/lib.rs").is_empty());
        assert_eq!(
            paths("cd crates/x && cat > /repo/src/lib.rs"),
            vec!["/repo/src/lib.rs"]
        );
    }
}
