//! Prove file-read literals without treating every string in a program as a path.
use std::path::{Path, PathBuf};

pub(super) fn parent_paths(program: &str, directory: &Path) -> Vec<PathBuf> {
    let program = without_comments(program);
    let tokens = tokens(&program);
    let mut found = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let Token::Literal(literal) = token else {
            continue;
        };
        let Some([Token::Name(call), Token::Mark('(')]) = index
            .checked_sub(2)
            .and_then(|start| tokens.get(start..index))
        else {
            continue;
        };
        let direct = matches!(
            *call,
            "open" | "io.open" | "readFileSync" | "fs.readFileSync"
        ) && matches!(tokens.get(index + 1), Some(Token::Mark(')' | ',')));
        let path_read = matches!(*call, "Path" | "pathlib.Path")
            && matches!(
                tokens.get(index + 1..index + 5),
                Some([
                    Token::Mark(')'),
                    Token::Name(".read_text" | ".read_bytes"),
                    Token::Mark('('),
                    Token::Mark(')')
                ])
            );
        if direct || path_read {
            // Keep HEAD's extraction and normalization, restricted to the one
            // literal whose call establishes a read. Escapes are not evaluated.
            found.extend(crate::acceptance_sibling_paths::program_parent_paths(
                literal, directory,
            ));
        }
    }
    found
}

/// Comments cannot establish a read or a directory change. Keep quoted text
/// intact so comment markers inside a filename remain part of that literal.
pub(super) fn without_comments(program: &str) -> String {
    let mut out = String::with_capacity(program.len());
    let mut chars = program.chars().peekable();
    let mut quote = None;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == q {
                quote = None;
            }
        } else if matches!(c, '\'' | '"') {
            quote = Some(c);
            out.push(c);
        } else if c == '#' || (c == '/' && chars.peek() == Some(&'/')) {
            out.push(' ');
            for next in chars.by_ref() {
                if next == '\n' {
                    out.push(next);
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            out.push(' ');
            while let Some(next) = chars.next() {
                if next == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
                if next == '\n' {
                    out.push(next);
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Debug)]
enum Token<'a> {
    Name(&'a str),
    Literal(&'a str),
    Mark(char),
    Unknown,
}

fn tokens(program: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut chars = program.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if matches!(c, '\'' | '"') {
            let mut end = None;
            let mut escaped = false;
            while let Some((at, next)) = chars.next() {
                if next == '\\' {
                    escaped = true;
                    chars.next();
                } else if next == c {
                    end = Some(at + next.len_utf8());
                    break;
                }
            }
            tokens.push(match end.filter(|_| !escaped) {
                Some(end) => Token::Literal(&program[start..end]),
                None => Token::Unknown,
            });
        } else if c.is_ascii_alphabetic() || matches!(c, '_' | '.') {
            let mut end = start + c.len_utf8();
            while let Some(&(at, next)) = chars.peek() {
                if !next.is_ascii_alphanumeric() && !matches!(next, '_' | '.') {
                    break;
                }
                end = at + next.len_utf8();
                chars.next();
            }
            tokens.push(Token::Name(&program[start..end]));
        } else {
            tokens.push(Token::Mark(c));
        }
    }
    tokens
}
