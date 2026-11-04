use super::*;

/// Parent operands read from a proven logical directory, relative to the selected root.
pub(crate) fn scan_from(command: &str, directory: &std::path::Path) -> Vec<SiblingPath> {
    let command = strip_quote_prefixes(command);
    // R2: comments set aside, so an apostrophe in one opens no quote that
    // would hide every path after it.
    let (shell, stdin_flag) = crate::check_source_resolve::with_heredoc_programs(&command);
    let shell = without_comments(&shell);
    let words = crate::check_source_resolve::words(&shell);
    let mut found = Vec::new();
    let mut command_start = 0;
    let initial_directory = directory.to_path_buf();
    let mut directory = Some(initial_directory.clone());
    let mut previous = None;
    let mut directories = Vec::new();
    let mut scopes = Vec::new();
    while command_start < words.len() {
        let end = (command_start..words.len())
            .find(|index| {
                matches!(
                    words[*index].as_str(),
                    ";" | "&&" | "||" | "|" | "&" | "(" | ")"
                )
            })
            .unwrap_or(words.len());
        let segment = &words[command_start..end];
        let current = directory.clone();
        let at = current.as_deref();
        let mut redirect_target = false;
        let mut writes = Vec::new();
        let command_index = segment
            .iter()
            .position(|word| !is_assignment(word) && !word.starts_with("-"));
        let executable = command_index.map(|index| segment[index].as_str());
        let executable = executable
            .unwrap_or_default()
            .rsplit('/')
            .next()
            .unwrap_or_default();
        let mut required_indexes = Vec::new();
        if matches!(executable, "cd" | "pushd")
            && let Some(index) = command_index.and_then(|command| {
                ((command + 1)..segment.len()).find(|index| !segment[*index].starts_with('-'))
            })
        {
            required_indexes.push(index);
        }
        for index in 0..segment.len() {
            let word = &segment[index];
            if matches!(word.as_str(), "-C" | "--manifest-path" | "--path") {
                if index + 1 < segment.len() {
                    required_indexes.push(index + 1);
                }
            } else if let Some((option, value)) = word.split_once('=')
                && matches!(option, "-C" | "--manifest-path" | "--path")
                && !value.is_empty()
            {
                if let Some(path) = at.and_then(|directory| sibling_word(value, directory)) {
                    push(&mut found, path, true);
                }
            }
            if word == "<" && index + 1 < segment.len() {
                required_indexes.push(index + 1);
            }
            if matches!(word.as_str(), ">" | ">>" | "2>" | "1>") {
                redirect_target = true;
            } else if redirect_target {
                writes.push(index);
                redirect_target = false;
            }
            if executable == "tee" && command_index.is_some_and(|at| index > at) {
                writes.push(index);
            }
        }
        if matches!(executable, "test" | "[") {
            for (index, word) in segment.iter().enumerate() {
                if matches!(word.as_str(), "-f" | "-d" | "-e" | "-x" | "-s")
                    && index + 1 < segment.len()
                {
                    required_indexes.push(index + 1);
                }
            }
        }
        // Policy callers also pass quoted path-only commands. A program path
        // is an input for site copying even though mutation never moves it.
        for (index, word) in segment.iter().enumerate() {
            if (Some(index) != command_index || word.contains('/'))
                && !word.starts_with('-')
                && let Some(path) = at.and_then(|directory| sibling_word(word, directory))
            {
                push(
                    &mut found,
                    path,
                    !writes.contains(&index) && required_indexes.contains(&index),
                );
            }
        }
        for (_, _, program) in inline_programs(&[segment.to_vec()]) {
            if let Some(directory) = at {
                for path in program_parent_paths(&program, directory) {
                    push(&mut found, path, true);
                }
            }
        }
        for pair in segment.windows(2).filter(|pair| pair[0] == stdin_flag) {
            if let Some(directory) = at {
                for path in program_parent_paths(&pair[1], directory) {
                    push(&mut found, path, true);
                }
            }
        }
        // Recognize the common two-step lexical escape without evaluating shell.
        // Directory state is scoped just as shell directory changes are.
        let isolated = matches!(words.get(end).map(String::as_str), Some("|" | "&"))
            || command_start
                .checked_sub(1)
                .is_some_and(|i| words[i] == "|");
        if !isolated {
            update_directory(
                executable,
                segment,
                command_index,
                &mut directory,
                &mut previous,
                &mut directories,
            );
        }
        match words.get(end).map(String::as_str) {
            Some("(") => scopes.push((directory.clone(), previous.clone(), directories.clone())),
            Some(")") => {
                if let Some((cwd, old, stack)) = scopes.pop() {
                    directory = cwd;
                    previous = old;
                    directories = stack;
                }
            }
            _ => {}
        }
        command_start = end + 1;
    }
    found.sort_by(|a, b| a.path.cmp(&b.path).then(a.required.cmp(&b.required)));
    let mut merged: Vec<SiblingPath> = Vec::new();
    for item in found {
        if let Some(existing) = merged.iter_mut().find(|old| old.path == item.path) {
            existing.required |= item.required;
        } else {
            merged.push(item);
        }
    }
    merged
}

fn update_directory(
    executable: &str,
    segment: &[String],
    index: Option<usize>,
    directory: &mut Option<PathBuf>,
    previous: &mut Option<PathBuf>,
    stack: &mut Vec<Option<PathBuf>>,
) {
    if executable == "popd" {
        *previous = directory.clone();
        *directory = stack.pop().flatten();
        return;
    }
    if !matches!(executable, "cd" | "pushd") {
        return;
    }
    let operand = index.and_then(|at| {
        segment[at + 1..]
            .iter()
            .find(|word| !matches!(word.as_str(), "-P" | "-L" | "--"))
    });
    let next = match operand.map(String::as_str) {
        Some("-") => previous.clone(),
        Some(path) if !path.contains(['$', '~', '`', '*', '?']) => directory
            .as_ref()
            .map(|cwd| relative_normalize(cwd.join(path))),
        _ => None,
    };
    if executable == "pushd" {
        stack.push(directory.clone());
    }
    *previous = directory.clone();
    *directory = next;
}
