//! Source lines with comments and literal contents removed, for the brace
//! scanner.
//!
//! Braces and branch words inside a string, character literal or comment are
//! not code. Counting them desynchronised the scanner: one format string
//! such as `"{:#?}"` (whose `#` the old line-based comment stripper also
//! took for a comment) left a `{` unclosed, and that function and every
//! later one in the file went unscored. Each line is returned with its
//! comments dropped and every literal reduced to its delimiters (`""`,
//! `''`), keeping the line count, so line numbers still point at the source.

/// Comment and literal syntax, chosen by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Syntax {
    /// `//` and nested `/* */` comments; multi-line `"..."`, raw
    /// `r#"..."#`; `'x'` character literals told apart from `'a` lifetimes.
    Rust,
    /// `//` and `/* */` comments; `"..."` and `'...'` end at the line end;
    /// `"""..."""` and backtick literals may span lines. `preprocessor`
    /// drops `#` directive lines (C, C++, C#, Swift).
    CFamily { preprocessor: bool },
    /// `#` comments; `"..."` and `'...'`, plus multi-line `"""` / `'''`.
    Hash,
}

pub(super) fn syntax_for(path: &str) -> Syntax {
    let ext = path.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    match ext {
        "rs" => Syntax::Rust,
        "py" | "pyi" | "sh" => Syntax::Hash,
        "c" | "cc" | "cpp" | "h" | "hpp" | "cs" | "swift" => Syntax::CFamily { preprocessor: true },
        _ => Syntax::CFamily {
            preprocessor: false,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Code,
    /// Inside `/* */`, at this nesting depth.
    Block(u32),
    /// Inside a quoted literal closed by `close` repeated `width` times.
    Quoted {
        close: char,
        width: usize,
        escapes: bool,
        multiline: bool,
    },
    /// Inside a Rust raw string closed by `"` and this many `#`.
    Raw(usize),
}

/// `text`'s lines (as [`str::lines`] splits them) with comments removed and
/// literal contents dropped.
pub(super) fn code_lines(text: &str, syntax: Syntax) -> Vec<String> {
    let mut state = State::Code;
    text.lines()
        .map(|line| code_line(line, syntax, &mut state))
        .collect()
}

fn code_line(line: &str, syntax: Syntax, state: &mut State) -> String {
    let mut out = String::new();
    if *state == State::Code
        && syntax == (Syntax::CFamily { preprocessor: true })
        && line.trim_start().starts_with('#')
    {
        return out;
    }
    let chars: Vec<char> = line.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        at = match *state {
            State::Code => code_char(&chars, at, syntax, state, &mut out),
            State::Block(depth) => block_char(&chars, at, depth, syntax, state),
            State::Quoted {
                close,
                width,
                escapes,
                ..
            } => quoted_char(&chars, at, (close, width, escapes), state, &mut out),
            State::Raw(hashes) => raw_char(&chars, at, hashes, state, &mut out),
        };
    }
    if let State::Quoted {
        close,
        multiline: false,
        ..
    } = *state
    {
        out.push(close);
        *state = State::Code;
    }
    out
}

fn starts(chars: &[char], at: usize, text: &str) -> bool {
    text.chars()
        .enumerate()
        .all(|(offset, ch)| chars.get(at + offset) == Some(&ch))
}

fn ident_char(ch: Option<&char>) -> bool {
    ch.is_some_and(|ch| ch.is_alphanumeric() || *ch == '_')
}

/// One step in code; returns the next index.
fn code_char(
    chars: &[char],
    at: usize,
    syntax: Syntax,
    state: &mut State,
    out: &mut String,
) -> usize {
    let ch = chars[at];
    let hash = syntax == Syntax::Hash;
    if (!hash && starts(chars, at, "//")) || (hash && ch == '#') {
        return chars.len();
    }
    if !hash && starts(chars, at, "/*") {
        out.push(' ');
        *state = State::Block(1);
        return at + 2;
    }
    if syntax == Syntax::Rust
        && let Some(next) = raw_string_open(chars, at, state)
    {
        out.push('"');
        return next;
    }
    match ch {
        '"' | '\'' | '`' => open_quote(chars, at, syntax, state, out),
        _ => {
            out.push(ch);
            at + 1
        }
    }
}

/// `r"`, `r#"`, `br##"` ... at `at`, not inside an identifier.
fn raw_string_open(chars: &[char], at: usize, state: &mut State) -> Option<usize> {
    let r = if chars[at] == 'b' { at + 1 } else { at };
    if chars.get(r) != Some(&'r') || (at > 0 && ident_char(chars.get(at - 1))) {
        return None;
    }
    let hashes = chars[r + 1..].iter().take_while(|ch| **ch == '#').count();
    (chars.get(r + 1 + hashes) == Some(&'"')).then(|| {
        *state = State::Raw(hashes);
        r + 2 + hashes
    })
}

fn open_quote(
    chars: &[char],
    at: usize,
    syntax: Syntax,
    state: &mut State,
    out: &mut String,
) -> usize {
    let ch = chars[at];
    if syntax == Syntax::Rust && ch == '\'' {
        return rust_quote(chars, at, out);
    }
    if syntax == Syntax::Rust && ch == '`' {
        out.push(ch);
        return at + 1;
    }
    let triple = ch != '`' && starts(chars, at, &ch.to_string().repeat(3));
    let width = if triple { 3 } else { 1 };
    *state = State::Quoted {
        close: ch,
        width,
        escapes: ch != '`',
        multiline: triple || ch == '`' || syntax == Syntax::Rust,
    };
    out.push(ch);
    at + width
}

/// A Rust `'`: a character literal (`'x'`, `'\n'`, `'\''`) becomes `''`; a
/// lifetime or label (`'a`, `'outer:`) is kept as code.
fn rust_quote(chars: &[char], at: usize, out: &mut String) -> usize {
    let end = if chars.get(at + 1) == Some(&'\\') {
        (at + 3..chars.len()).find(|index| chars[*index] == '\'')
    } else if chars.get(at + 2) == Some(&'\'') {
        Some(at + 2)
    } else {
        None
    };
    match end {
        Some(end) => {
            out.push_str("''");
            end + 1
        }
        None => {
            out.push('\'');
            at + 1
        }
    }
}

fn block_char(chars: &[char], at: usize, depth: u32, syntax: Syntax, state: &mut State) -> usize {
    if starts(chars, at, "*/") {
        *state = if depth <= 1 {
            State::Code
        } else {
            State::Block(depth - 1)
        };
        return at + 2;
    }
    if syntax == Syntax::Rust && starts(chars, at, "/*") {
        *state = State::Block(depth + 1);
        return at + 2;
    }
    at + 1
}

fn quoted_char(
    chars: &[char],
    at: usize,
    (close, width, escapes): (char, usize, bool),
    state: &mut State,
    out: &mut String,
) -> usize {
    if escapes && chars[at] == '\\' {
        return at + 2;
    }
    if starts(chars, at, &close.to_string().repeat(width)) {
        out.push(close);
        *state = State::Code;
        return at + width;
    }
    at + 1
}

fn raw_char(
    chars: &[char],
    at: usize,
    hashes: usize,
    state: &mut State,
    out: &mut String,
) -> usize {
    let closes =
        chars[at] == '"' && (1..=hashes).all(|offset| chars.get(at + offset) == Some(&'#'));
    if !closes {
        return at + 1;
    }
    out.push('"');
    *state = State::Code;
    at + 1 + hashes
}
