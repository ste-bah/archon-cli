use super::*;

/// The first of `forms` that `text` names, both compared in `spelling`.
pub(super) fn named_root_in<'a>(
    spelling: Spelling,
    text: &str,
    forms: &'a [PathBuf],
) -> Option<&'a PathBuf> {
    let text = spelling.fold(text);
    let mut environment = std::collections::HashMap::<String, String>::new();
    let mut expanded_words = Vec::new();
    let mut carry_from_previous = true;
    for (segment, follows_pipeline, carry_assignments_forward) in
        shell_command_segments(&text, spelling)
    {
        if !carry_from_previous && !environment.is_empty() {
            environment.clear();
        }
        let words = shell_words(segment, spelling);
        let assignments: Vec<_> = words.iter().map(|word| shell_assignment(word)).collect();
        let mut assignment_positions = Vec::new();
        let mut prefix_end = 0;
        while prefix_end < words.len() && assignments[prefix_end].is_some() {
            assignment_positions.push(prefix_end);
            prefix_end += 1;
        }
        let command = words.get(prefix_end).map(String::as_str);
        let assignment_builtin = matches!(command, Some("export" | "readonly" | "local"));
        if assignment_builtin {
            assignment_positions.extend(
                assignments
                    .iter()
                    .enumerate()
                    .skip(prefix_end + 1)
                    .filter_map(|(index, assignment)| assignment.as_ref().map(|_| index)),
            );
        } else if prefix_end == words.len() {
            // A segment made entirely of assignment words is a standalone
            // assignment statement; its variables remain available later.
        } else {
            assignment_positions.truncate(prefix_end);
        }

        let persists = (prefix_end == words.len() || assignment_builtin)
            && !follows_pipeline
            && carry_assignments_forward;
        let mut segment_values = environment.clone();
        for &index in &assignment_positions {
            if let Some((name, value)) = &assignments[index] {
                segment_values.insert(name.clone(), value.clone());
            }
        }
        for (index, word) in words.iter().enumerate() {
            if assignment_positions.contains(&index) {
                continue;
            }
            expanded_words.push(segment_values.iter().fold(
                word.clone(),
                |value, (name, assigned)| {
                    value
                        .replace(&format!("${{{name}}}"), assigned)
                        .replace(&format!("${name}"), assigned)
                },
            ));
        }
        for word in &words {
            for nested in command_substitution_words(word, spelling) {
                expanded_words.push(segment_values.iter().fold(
                    nested,
                    |value, (name, assigned)| {
                        value
                            .replace(&format!("${{{name}}}"), assigned)
                            .replace(&format!("${name}"), assigned)
                    },
                ));
            }
        }
        if persists {
            for index in assignment_positions {
                if let Some((name, value)) = &assignments[index] {
                    environment.insert(name.clone(), value.clone());
                }
            }
        }
        carry_from_previous = carry_assignments_forward;
    }
    let paths: Vec<PathBuf> = expanded_words
        .iter()
        .filter_map(|word| lexical(word))
        .collect();
    let textual = forms.iter().find(|form| {
        let root = spelling.fold(&form.to_string_lossy());
        let form = Path::new(&root);
        !root.is_empty()
            && (paths.iter().any(|path| path.starts_with(form))
                || spelling == Spelling::Posix
                    && expanded_words.iter().any(|word| {
                        word.strip_prefix(&root).is_some_and(|suffix| {
                            suffix.is_empty()
                                || suffix
                                    .chars()
                                    .next()
                                    .is_some_and(|next| !continues_name(next))
                        })
                    })
                || spelling == Spelling::Windows
                    && expanded_words
                        .iter()
                        .any(|word| windows_path_word_names(word, &root)))
    });
    textual.or_else(|| {
        forms
            .iter()
            .find(|form| paths.iter().any(|path| names_by_identity(path, form)))
    })
}

/// A literal shell assignment word has a NAME on the left side and appears
/// before the command name (or as an assignment-only command).
fn shell_assignment(word: &str) -> Option<(String, String)> {
    let (name, value) = word.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    (first == '_' || first.is_ascii_alphabetic()).then_some(())?;
    chars
        .all(|character| character == '_' || character.is_ascii_alphanumeric())
        .then(|| (name.to_owned(), value.to_owned()))
}

/// Include literal words inside `$(...)` for path inspection without running
/// shell code. This preserves detection of a root used to build a path while
/// keeping prose and ordinary arguments outside the substitution in scope.
fn command_substitution_words(text: &str, spelling: Spelling) -> Vec<String> {
    let mut result = Vec::new();
    let mut offset = 0;
    while let Some(relative) = text[offset..].find("$(") {
        let start = offset + relative;
        let Some(end) = command_substitution_end(text, start, spelling) else {
            break;
        };
        let inner_start = start + 2;
        let inner = &text[inner_start..end];
        result.extend(shell_words(inner, spelling));
        result.extend(command_substitution_words(inner, spelling));
        offset = end + 1;
    }
    result
}

fn command_substitution_end(text: &str, start: usize, spelling: Spelling) -> Option<usize> {
    let mut depth = 1usize;
    let mut quote = None;
    let mut escaped = false;
    let mut index = start + 2;
    while index < text.len() {
        let ch = text[index..].chars().next()?;
        let next = index + ch.len_utf8();
        if escaped {
            escaped = false;
        } else if spelling == Spelling::Posix && quote != Some('\'') && ch == '\\' {
            escaped = true;
        } else if let Some(current) = quote {
            if ch == current {
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
        index = next;
    }
    None
}

fn windows_path_word_names(word: &str, root: &str) -> bool {
    let word = word.to_lowercase().replace('/', "\\");
    let root = root.to_lowercase().replace('/', "\\");
    word.strip_prefix(&root).is_some_and(|suffix| {
        suffix.is_empty()
            || suffix.starts_with('\\')
            || suffix
                .chars()
                .next()
                .is_some_and(|next| !continues_name(next))
    })
}

/// Read shell words with quote removal and adjacent-fragment concatenation.
/// Variable expansion is intentionally limited to recognized literal shell
/// assignments; arbitrary shell evaluation is not attempted.
fn shell_words(text: &str, spelling: Spelling) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in text.chars() {
        if escaped {
            word.push(ch);
            escaped = false;
        } else if spelling == Spelling::Posix && quote != Some('\'') && ch == '\\' {
            escaped = true;
        } else if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                word.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch.is_whitespace() || ";|&<>()".contains(ch) {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Split shell command lists without treating separators inside quotes as
/// command boundaries. Standalone assignments can feed later segments.
fn shell_command_segments(text: &str, spelling: Spelling) -> Vec<(&str, bool, bool)> {
    let mut segments = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut start = 0;
    let mut follows_pipeline = false;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if escaped {
            escaped = false;
        } else if spelling == Spelling::Posix && quote != Some('\'') && ch == '\\' {
            escaped = true;
        } else if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ";|&\n".contains(ch) {
            let paired =
                matches!(ch, '&' | '|') && chars.peek().is_some_and(|(_, next)| *next == ch);
            let mut end = index + ch.len_utf8();
            if paired {
                end += chars.next().map_or(0, |(_, next)| next.len_utf8());
            }
            let carries = ch == ';' || ch == '\n' || paired;
            segments.push((&text[start..index], follows_pipeline, carries));
            start = end;
            follows_pipeline = ch == '|' && !paired;
        }
    }
    segments.push((&text[start..], follows_pipeline, false));
    segments
}
