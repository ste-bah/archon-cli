//! Keep named inputs relative to the directory in which their command reads them.
use std::path::{Component, Path, PathBuf};
#[path = "task_set_contract_policy_paths_argv.rs"]
mod argv;
#[path = "task_set_contract_policy_paths_program.rs"]
mod program;

pub(super) fn path_command(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| quote(&path.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

pub(super) fn parent_paths(command: &str, root: &Path) -> Vec<PathBuf> {
    let mut found = program_paths(command, root, Some(PathBuf::new()));
    found.retain(|path| path.components().any(|part| part == Component::ParentDir));
    found.sort();
    found.dedup();
    found
}

fn program_paths(command: &str, root: &Path, mut directory: Option<PathBuf>) -> Vec<PathBuf> {
    let (shell, heredoc_flag) = crate::check_source_resolve::with_heredoc_programs(command);
    let shell = crate::acceptance_sibling_paths::without_comments(&shell);
    let (mut words, mut literal, mut operators) = (Vec::new(), Vec::new(), Vec::new());
    for (word, certain, operator) in crate::check_source_resolve::words_with_certainty(&shell) {
        words.push(word);
        literal.push(certain);
        operators.push(operator);
    }
    let mut found = Vec::new();
    let mut start = 0;
    while start < words.len() {
        let end = (start..words.len())
            .find(|index| operators[*index] && is_operator(&words[*index]))
            .unwrap_or(words.len());
        let segment = &words[start..end];
        // Flags such as `test -e` and `cat -e` take ordinary operands, not
        // inline programs. Only an interpreter can give these flags that role.
        let mut programs = if argv::target(segment)
            .and_then(|command| command.first())
            .is_some_and(|name| argv::interpreter(name))
        {
            crate::acceptance_sibling_paths::inline_programs(&[segment.to_vec()])
        } else {
            Vec::new()
        };
        programs.extend(segment.windows(2).enumerate().filter_map(|(index, pair)| {
            (pair[0] == heredoc_flag).then(|| (0, index + 1, pair[1].clone()))
        }));
        // Program operands have their own grammar and directory state. They must
        // neither be read as outer operands nor change the caller's directory.
        let mut outside = segment.to_vec();
        let mut certain = literal[start..end].to_vec();
        for (_, index, _) in &programs {
            outside[*index].clear();
            certain[*index] = true;
        }
        let next = words.get(end).map(String::as_str);
        let changes_directory = changes_directory(&outside);
        let operand = plain_directory(&outside);
        // A literal directory target is read from the current directory before
        // the change can make subsequent relative operands unknown.
        if !compound_syntax(&outside)
            && !certain.contains(&false)
            && !next.is_some_and(unknown_operator)
            && directory_target(&outside).is_some()
            && let Some(base) = directory.as_deref()
        {
            found.extend(scan_segment(&outside, &operators[start..end], base));
        }
        // Only simple segments and plain literal cd steps prove a directory.
        // Unsupported syntax makes it unknown permanently; never evaluate branches.
        if compound_syntax(&outside)
            || certain.contains(&false)
            || (changes_directory && (operand.is_none() || next.is_some_and(unknown_operator)))
            || directory.as_ref().is_some_and(|base| {
                operand.is_some_and(|operand| !real_directory(&root.join(base), operand))
            })
        {
            directory = None;
        }
        let visible = argv::operands(&outside, root, directory.as_deref());
        found.extend(nested_paths(
            segment,
            &programs,
            &literal[start..end],
            root,
            directory.clone().filter(|_| !visible.is_empty()),
            visible.len(),
            &heredoc_flag,
        ));
        if let Some(base) = directory.clone() {
            found.extend(scan_segment(
                visible,
                &operators[start..start + visible.len()],
                &base,
            ));
            if let Some(operand) = operand {
                let target = logical(&base.join(operand));
                // Every component was proven a real directory, so logical and physical
                // arithmetic agree. Never canonicalize `..` through a symlink.
                directory = Some(target);
            }
        }
        if next.is_some_and(unknown_operator) {
            directory = None;
        }
        start = end + 1;
    }
    found
}

fn scan_segment(segment: &[String], operators: &[bool], base: &Path) -> Vec<PathBuf> {
    let paths = argv::path_operands(segment, operators);
    let text = paths
        .iter()
        .map(|word| quote(word))
        .collect::<Vec<_>>()
        .join(" ");
    crate::acceptance_sibling_paths::scan_from(&text, base)
        .into_iter()
        .map(|named| named.path)
        .collect()
}

fn nested_paths(
    segment: &[String],
    programs: &[(usize, usize, String)],
    literal: &[bool],
    root: &Path,
    directory: Option<PathBuf>,
    visible: usize,
    heredoc_flag: &str,
) -> Vec<PathBuf> {
    let unwrapped = argv::target(segment).unwrap_or_default();
    let shell = unwrapped
        .first()
        .is_some_and(|name| matches!(name.as_str(), "sh" | "bash" | "dash" | "zsh" | "ksh"));
    let interpreter = unwrapped
        .first()
        .is_some_and(|name| argv::interpreter(name));
    let mut found = Vec::new();
    for (_, index, program) in programs {
        if *index >= visible {
            continue;
        }
        let base = directory.clone().filter(|_| literal[*index]);
        if shell {
            // A child inherits the current directory; its cd never leaks back.
            found.extend(program_paths(program, root, base));
        } else if (interpreter
            || (segment
                .get(index - 1)
                .is_some_and(|flag| flag == heredoc_flag)
                && !unwrapped.first().is_some_and(|name| data_consumer(name))))
            && let Some(base) = base.filter(|_| interpreter_directory_known(program))
        {
            // Reuse the original program-text detection. Ordinary argv remains
            // opaque, and unproven directory changes never prove a relative read.
            found.extend(program::parent_paths(program, &base));
        }
    }
    found
}

fn data_consumer(name: &str) -> bool {
    matches!(
        name,
        "cat"
            | "tee"
            | "grep"
            | "egrep"
            | "fgrep"
            | "rg"
            | "sed"
            | "awk"
            | "wc"
            | "head"
            | "tail"
            | "sort"
            | "uniq"
            | "cut"
            | "tr"
            | "diff"
            | "cmp"
    )
}

fn interpreter_directory_known(program: &str) -> bool {
    let program = program::without_comments(program);
    let words = crate::check_source_resolve::words(&program);
    // An unfamiliar call may change the directory or dispatch hidden code.
    // Only direct file inspection calls prove enough to scan relative strings.
    let known_calls = words.windows(2).all(|pair| {
        pair[1] != "("
            || matches!(
                pair[0].as_str(),
                "open"
                    | "io.open"
                    | "Path"
                    | "pathlib.Path"
                    | ".read_text"
                    | ".read_bytes"
                    | "readFileSync"
                    | "fs.readFileSync"
                    | "read"
                    | "read_text"
                    | "read_bytes"
                    | "exists"
                    | "is_file"
                    | "is_dir"
                    | "stat"
                    | "access"
                    | "print"
            )
    });
    known_calls
        && !program
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|word| {
                matches!(
                    word,
                    "cd" | "pushd"
                        | "popd"
                        | "chdir"
                        | "fchdir"
                        | "chroot"
                        | "setwd"
                        | "eval"
                        | "exec"
                        | "getattr"
                        | "send"
                        | "system"
                        | "popen"
                        | "subprocess"
                        | "__import__"
                        | "import"
                        | "from"
                        | "require"
                        | "include"
                        | "load"
                )
            })
        && !program.contains(['$', '`'])
}

fn is_operator(word: &str) -> bool {
    matches!(word, ";" | "&&" | "||" | "|" | "&" | "(" | ")")
}

fn unknown_operator(word: &str) -> bool {
    matches!(word, "||" | "|" | "&" | "(" | ")")
}

fn changes_directory(segment: &[String]) -> bool {
    segment
        .iter()
        .any(|word| matches!(word.as_str(), "cd" | "pushd" | "popd"))
}

fn compound_syntax(segment: &[String]) -> bool {
    segment.iter().any(|word| {
        matches!(
            word.as_str(),
            "!" | "if"
                | "then"
                | "elif"
                | "else"
                | "fi"
                | "for"
                | "while"
                | "until"
                | "do"
                | "done"
                | "case"
                | "esac"
                | "in"
                | "select"
                | "function"
                | "time"
                | "coproc"
                | "{"
                | "}"
        )
    })
}

fn real_directory(base: &Path, operand: &Path) -> bool {
    let mut path = base.to_path_buf();
    operand.components().all(|part| {
        path.push(part);
        // symlink_metadata does not follow the component being checked. Checking
        // every step also catches a link followed by `..` before logical reduction.
        std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_dir())
    })
}

fn plain_directory(segment: &[String]) -> Option<&Path> {
    (segment.first().is_some_and(|command| command == "cd"))
        .then(|| directory_target(segment))
        .flatten()
}

fn directory_target(segment: &[String]) -> Option<&Path> {
    let [command, operand] = segment else {
        return None;
    };
    let path = Path::new(operand);
    (matches!(command.as_str(), "cd" | "pushd")
        && !operand.starts_with(['-', '~'])
        && !path.is_absolute()
        && !operand.contains(['$', '`', '*', '?', '[', ']', '{', '}']))
    .then_some(path)
}

fn logical(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir
                if matches!(result.components().next_back(), Some(Component::Normal(_))) =>
            {
                result.pop();
            }
            part => result.push(part),
        }
    }
    result
}
