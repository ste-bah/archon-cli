use std::collections::BTreeMap;

use super::{GROUP_CLOSE, GROUP_OPEN, SUBSHELL_CLOSE, SUBSHELL_OPEN};

pub(crate) fn group_has_exit(commands: &[Vec<String>], start: usize) -> bool {
    let mut depth = 0usize;
    for words in commands.iter().skip(start) {
        if words.iter().any(|word| word == GROUP_OPEN) {
            depth += 1;
        }
        let command = words
            .iter()
            .find(|word| !word.starts_with('\0') && !matches!(word.as_str(), "then" | "do"));
        if command.is_some_and(|word| matches!(word.as_str(), "exit" | "return")) {
            return true;
        }
        if words.iter().any(|word| word == GROUP_CLOSE) {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return false;
            }
        }
    }
    false
}

pub(super) fn substitute_leading_variables(
    word: &str,
    variables: &BTreeMap<String, String>,
) -> String {
    let (name, suffix) = if let Some(rest) = word.strip_prefix("${") {
        let Some(end) = rest.find('}') else {
            return word.to_string();
        };
        (&rest[..end], &rest[end + 1..])
    } else if let Some(rest) = word.strip_prefix('$') {
        let end = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            .count();
        if end == 0 {
            return word.to_string();
        }
        (&rest[..end], &rest[end..])
    } else {
        return word.to_string();
    };
    variables
        .get(name)
        .map_or_else(|| word.to_string(), |value| format!("{value}{suffix}"))
}

/// Shell words by simple command plus here-doc bodies and their command index.
/// Quote-aware and deliberately best effort.
pub fn lex(text: &str) -> (Vec<Vec<String>>, Vec<(usize, String)>) {
    let (mut commands, mut bodies) = (vec![Vec::new()], Vec::new());
    let (mut word, mut quote, mut escaped, mut pending_tags): (
        String,
        Option<char>,
        bool,
        Vec<(String, bool, usize)>,
    ) = (String::new(), None, false, Vec::new());
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
            if escaped {
                word.push(c);
                escaped = false;
                i += 1;
                continue;
            }
            if quote != Some('\'') && c == '\\' {
                escaped = true;
                i += 1;
                continue;
            }
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => word.push(c),
                None if c == '\'' || c == '"' => quote = Some(c),
                None if c == '<' && chars.get(i + 1) == Some(&'<') => {
                    finish(&mut word, &mut commands);
                    i += 2;
                    let strip_tabs = chars.get(i) == Some(&'-');
                    if strip_tabs {
                        i += 1;
                    }
                    while chars.get(i).is_some_and(|c| c.is_whitespace()) {
                        i += 1;
                    }
                    let mut tag = String::new();
                    let mut delimiter_quote = None;
                    let mut delimiter_escaped = false;
                    while let Some(&delimiter_char) = chars.get(i) {
                        if delimiter_escaped {
                            tag.push(delimiter_char);
                            delimiter_escaped = false;
                        } else if delimiter_quote != Some('\'') && delimiter_char == '\\' {
                            delimiter_escaped = true;
                            i += 1;
                            continue;
                        } else if let Some(q) = delimiter_quote {
                            if delimiter_char == q {
                                delimiter_quote = None;
                            } else {
                                tag.push(delimiter_char);
                            }
                        } else if delimiter_char == '\'' || delimiter_char == '"' {
                            delimiter_quote = Some(delimiter_char);
                        } else if delimiter_char.is_whitespace()
                            || matches!(delimiter_char, ';' | '&' | '|' | '(' | ')' | '>' | '<')
                        {
                            break;
                        } else {
                            tag.push(delimiter_char);
                        }
                        i += 1;
                    }
                    if !tag.is_empty() {
                        pending_tags.push((tag, strip_tabs, commands.len() - 1));
                    }
                    i = i.saturating_sub(1);
                }
                None if matches!(c, '>' | '<') => {
                    finish(&mut word, &mut commands);
                    let operator = if chars.get(i + 1) == Some(&c) {
                        i += 1;
                        format!("{c}{c}")
                    } else {
                        c.to_string()
                    };
                    commands.last_mut().expect("one command").push(operator);
                }
                None if c == '#'
                    && (i == 0
                        || chars[i - 1].is_whitespace()
                        || ";&|(){}".contains(chars[i - 1]))
                    && !(i > 0 && chars[i - 1] == '$')
                    && !(i > 1 && chars[i - 1] == '{' && chars[i - 2] == '$') =>
                {
                    break;
                }
                None if c.is_whitespace() => finish(&mut word, &mut commands),
                None if c == '{' => {
                    finish(&mut word, &mut commands);
                    let carried = commands
                        .last_mut()
                        .filter(|words| {
                            words
                                .iter()
                                .all(|word| matches!(word.as_str(), "\0and" | "\0or"))
                        })
                        .map(std::mem::take)
                        .unwrap_or_default();
                    commands.push(carried);
                    commands
                        .last_mut()
                        .expect("group command")
                        .push(GROUP_OPEN.to_string());
                }
                None if c == '}' => {
                    finish(&mut word, &mut commands);
                    commands
                        .last_mut()
                        .expect("one command")
                        .push(GROUP_CLOSE.to_string());
                    commands.push(Vec::new());
                }
                None if matches!(c, ';' | '&' | '|' | '(' | ')' | '{' | '}' | '`') => {
                    finish(&mut word, &mut commands);
                    let conditional = if matches!(c, '&' | '|') && chars.get(i + 1) == Some(&c) {
                        i += 1;
                        Some(if c == '&' { "\0and" } else { "\0or" })
                    } else {
                        None
                    };
                    if c == '(' || c == ')' {
                        commands.last_mut().expect("one command").push(
                            if c == '(' {
                                SUBSHELL_OPEN
                            } else {
                                SUBSHELL_CLOSE
                            }
                            .to_string(),
                        );
                    }
                    commands.push(Vec::new());
                    if let Some(marker) = conditional {
                        commands
                            .last_mut()
                            .expect("new command")
                            .push(marker.to_string());
                    }
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
        if quote.is_some() || escaped {
            if quote.is_some() && !escaped {
                word.push('\n');
            }
            continue;
        }
        finish(&mut word, &mut commands);
        commands.push(Vec::new());
        for (tag, strip_tabs, command_index) in std::mem::take(&mut pending_tags) {
            let mut body = String::new();
            for line in lines.by_ref() {
                let compared = if strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line
                };
                if compared == tag {
                    break;
                }
                body.push_str(compared);
                body.push('\n');
            }
            bodies.push((command_index, body));
        }
    }
    finish(&mut word, &mut commands);
    (commands, bodies)
}
