use regex::Regex;

use super::harness::WorkflowV2HarnessError;

pub(super) fn reject_unsafe_source(source: &str) -> Result<(), WorkflowV2HarnessError> {
    let lower = code_without_string_literals(source).to_ascii_lowercase();
    let blocked_patterns = [
        ("dynamic import", r#"\bimport\s*\("#),
        ("import statement", r#"\bimport\s*(?:["'\{\*\w]|\.)"#),
        ("provider literal", r#"\bprovider\s*:"#),
        ("model literal", r#"\bmodel\s*:"#),
    ];
    for (label, pattern) in blocked_patterns {
        let re = Regex::new(pattern).expect("blocked pattern compiles");
        if re.is_match(&lower) {
            return Err(WorkflowV2HarnessError::ForbiddenToken(label));
        }
    }
    for token in [
        "import ",
        "require(",
        "eval(",
        "new function",
        "function(",
        "fs.",
        "node:fs",
        "child_process",
        "process.",
        "deno.",
        "bun.",
        "fetch(",
        "xmlhttprequest",
        "websocket",
        "net.",
        "tls.",
        "http.",
        "https.",
        "anthropic",
        "openai",
        "claude-",
        "gpt-",
        "gemini",
        "provider:",
        "model:",
    ] {
        if lower.contains(token) {
            return Err(WorkflowV2HarnessError::ForbiddenToken(token));
        }
    }
    Ok(())
}

pub(super) fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut quote = None::<char>;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if let Some(active) = quote {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => {
                quote = Some(ch);
                out.push(ch);
            }
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                    }
                    if prev == '*' && next == '/' {
                        break;
                    }
                    prev = next;
                }
            }
            _ => out.push(ch),
        }
    }
    out
}

pub(super) fn code_without_string_literals(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let chars = source.chars();
    let mut quote = None::<char>;
    let mut escaped = false;
    for ch in chars {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
                out.push(ch);
            } else if ch == '\n' {
                out.push('\n');
            } else {
                out.push(' ');
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => {
                quote = Some(ch);
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}
