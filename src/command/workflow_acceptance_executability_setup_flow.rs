//! Shell status provenance for missing setup paths.

/// Commands and the following control operator, preserving quoted operators.
pub(crate) fn segments(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut body = String::new();
    let mut quote = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && quote != Some('\'') {
            body.push(c);
            if let Some(next) = chars.next() {
                body.push(next);
            }
        } else if quote == Some(c) {
            quote = None;
            body.push(c);
        } else if quote.is_some() {
            body.push(c);
        } else if matches!(c, '\'' | '"') {
            quote = Some(c);
            body.push(c);
        } else if c == '#' && body.chars().next_back().is_none_or(char::is_whitespace) {
            for next in chars.by_ref() {
                if next == '\n' {
                    break;
                }
            }
            found.push((std::mem::take(&mut body), ";".into()));
        } else if c == '&' && (body.ends_with(['>', '<']) || chars.peek() == Some(&'>')) {
            body.push(c);
        } else if matches!(c, ';' | '\n' | '&' | '|' | '(' | ')') {
            let mut op = if c == '\n' { ";".into() } else { c.to_string() };
            if matches!(c, '&' | '|') && chars.peek() == Some(&c) {
                op.push(chars.next().unwrap());
            }
            found.push((std::mem::take(&mut body), op));
        } else {
            body.push(c);
        }
    }
    found.push((body, String::new()));
    found
}

/// Whether failure of this simple command can determine the script status.
/// Guarded commands do not supply evidence that setup stopped the script.
pub(super) fn decisive(text: &str, word: &str, args: &[String]) -> bool {
    let parts = segments(text);
    let mut errexit = false;
    // On a failed positive condition only the else arm runs;
    // on a negated condition the then arm runs. A terminating
    // failure there is setup deciding the exit, not a guard.
    // Unmodeled compound control flow cannot establish a can-fail proof.
    // Inspect whole shell words, never substrings in filenames or arguments.
    let compound = super::super::super::verdict_shell::lex(text)
        .iter()
        .any(|words| {
            words.first().is_some_and(|word| {
                matches!(
                    word.as_str(),
                    "if" | "elif" | "else" | "while" | "until" | "for" | "case" | "function"
                )
            })
        })
        || parts.iter().any(|(_, op)| op == "(");
    if compound {
        return true;
    }
    let mut brace_depth = 0usize;
    for (index, (body, op)) in parts.iter().enumerate() {
        // Only leading brace keywords affect the command's enclosing group.
        for words in super::super::super::verdict_shell::lex(body) {
            for word in words
                .iter()
                .take_while(|word| matches!(word.as_str(), "{" | "}"))
            {
                if word == "{" {
                    brace_depth += 1;
                } else {
                    brace_depth = brace_depth.saturating_sub(1);
                }
            }
        }
        let isolated = matches!(op.as_str(), "|" | "&");
        let guarded = isolated;
        for command in super::super::super::verdict_shell::simple_commands(body) {
            if command.word == word && command.args == args {
                if brace_depth > 0 {
                    return true;
                }
                if isolated {
                    return false;
                }
                let mut end = index;
                while matches!(parts[end].1.as_str(), "&&" | "||" | "|") && end + 1 < parts.len() {
                    end += 1;
                }
                let last = parts[end + 1..]
                    .iter()
                    .all(|(body, _)| body.trim().is_empty());
                // A failed left operand skips &&, but executes ||. Only a
                // success guard masks its status; an explicit exit preserves it.
                let fallback = (index..end).find(|i| parts[*i].1 == "||");
                if let Some(i) = fallback {
                    let next = parts[i + 1].0.trim();
                    if next == "true" || next == ":" {
                        return false;
                    }
                    let fallback_end = if next.starts_with('{') {
                        (i + 1..parts.len())
                            .find(|j| parts[*j].0.contains('}'))
                            .unwrap_or(end)
                    } else {
                        end
                    };
                    let fallback = parts[i + 1..=fallback_end]
                        .iter()
                        .map(|(body, _)| body.as_str())
                        .collect::<Vec<_>>()
                        .join(";");
                    if failure_exit(&fallback) {
                        return true;
                    }
                    if next.starts_with("exit 0") || next.starts_with("return 0") {
                        return false;
                    }
                    // A printing fallback masks failure only when a later
                    // command decides the exit. Unknown fallbacks are not proof.
                    return !(next.starts_with("echo ") || next.starts_with("printf ")) || last;
                }
                return !guarded
                    && (last
                        || reads_list_status(&parts[end + 1..])
                        || (errexit && !matches!(op.as_str(), "&&" | "||")));
            }
            if command.word == "set" && !guarded {
                for arg in &command.args {
                    if arg.starts_with('-') && !arg.starts_with("--") && arg.contains('e') {
                        errexit = true;
                    }
                    if arg.starts_with('+') && arg.contains('e') {
                        errexit = false;
                    }
                }
                if command.args.windows(2).any(|a| a == ["-o", "errexit"]) {
                    errexit = true;
                }
                if command.args.windows(2).any(|a| a == ["+o", "errexit"]) {
                    errexit = false;
                }
            }
        }
    }
    false
}

/// A later exit refers to setup only if its list status is read immediately.
fn reads_list_status(parts: &[(String, String)]) -> bool {
    let words = parts
        .iter()
        .flat_map(|(body, _)| super::super::super::verdict_shell::lex(body))
        .collect::<Vec<_>>();
    let Some(first) = words.first() else {
        return false;
    };
    let exits_with = |words: &[String], value: &str| {
        words
            .first()
            .is_some_and(|word| matches!(word.as_str(), "exit" | "return"))
            && words.get(1).is_some_and(|arg| arg == value)
    };
    if exits_with(first, "$?") {
        return true;
    }
    let Some(name) = first.first().and_then(|word| word.strip_suffix("=$?")) else {
        return false;
    };
    // An assignment followed by a command reads that command's status instead.
    if first.len() != 1 || name.is_empty() {
        return false;
    }
    for command in &words[1..] {
        if command
            .iter()
            .any(|word| word.starts_with(&format!("{name}=")))
        {
            return false;
        }
        if exits_with(command, &format!("${name}")) || exits_with(command, &format!("${{{name}}}"))
        {
            return true;
        }
    }
    false
}

fn failure_exit(text: &str) -> bool {
    segments(text).iter().any(|(body, _)| {
        let body = body.trim().trim_start_matches(['{', '}']).trim();
        let body = body
            .strip_prefix("then ")
            .or_else(|| body.strip_prefix("else "))
            .unwrap_or(body);
        let mut words = body.split_whitespace();
        match words.next() {
            Some("exit" | "return") => words.next() != Some("0"),
            Some("false") => true,
            _ => false,
        }
    })
}

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_setup_flow_tests.rs"]
mod tests;
