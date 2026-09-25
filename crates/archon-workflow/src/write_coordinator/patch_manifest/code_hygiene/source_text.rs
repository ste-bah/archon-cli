//! Source lines with comments and literal contents removed, for the brace
//! scanner.
//!
//! Braces and branch words inside a string, character literal or comment are
//! not code. Counting them desynchronised the scanner: one format string
//! such as `"{:#?}"` (whose `#` the old line-based comment stripper also
//! took for a comment) left a `{` unclosed, and that function and every
//! later one in the file went unscored. Each line is returned twice: as
//! code, with comments dropped and every literal reduced to its delimiters
//! (`""`, `''`), and as kept text, with comments dropped but literal
//! contents intact, which is what a function's signature is compared by.
//! The line count is preserved, so line numbers still point at the source.
//!
//! A `'` or single-line `"` literal still open at the end of its line was
//! not a literal (an apostrophe in JSX text, a `/"/` regex, a `1'000` digit
//! separator): its opener is taken as code and the rest of the line
//! rescanned.

/// Comment and literal syntax, chosen by file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Syntax {
    /// `//` and nested `/* */` comments; multi-line `"..."`, raw
    /// `r#"..."#` / `br".."` / `cr".."`; `'x'` character literals told apart
    /// from `'a` lifetimes.
    Rust,
    /// `//` and `/* */` comments; `"..."` and `'...'` on one line;
    /// multi-line `"""..."""` and backtick literals, whose `\` escapes
    /// unless `raw_backticks` (Go). `preprocessor` drops `#` directive
    /// lines (C, C++, C#, Swift).
    CFamily {
        preprocessor: bool,
        raw_backticks: bool,
    },
    /// `#` comments; `"..."` and `'...'` on one line; multi-line `"""` and
    /// `'''`.
    Python,
    /// `#` comments only at the start of a word (so `${#a[@]}`, `${x#*:}`
    /// and `$#` are code); multi-line `"..."`; `'...'` without escapes.
    Shell,
}

pub(super) fn syntax_for(path: &str) -> Syntax {
    let ext = path.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    let c_family = |preprocessor, raw_backticks| Syntax::CFamily {
        preprocessor,
        raw_backticks,
    };
    match ext {
        "rs" => Syntax::Rust,
        "py" | "pyi" | "rb" | "rake" => Syntax::Python,
        "sh" => Syntax::Shell,
        "c" | "cc" | "cpp" | "cxx" | "h" | "hh" | "hpp" | "hxx" | "inl" | "ipp" | "tpp" | "cs"
        | "swift" => c_family(true, false),
        "go" => c_family(false, true),
        _ => c_family(false, false),
    }
}

/// One source line as the scanner reads it.
#[derive(Debug, Default)]
pub(super) struct CodeLine {
    /// Comments dropped, literal contents dropped.
    pub(super) code: String,
    /// Comments dropped, literal contents kept.
    kept: String,
    /// For each byte of `code`, the offset in `kept` of the char it is in.
    map: Vec<usize>,
}

impl CodeLine {
    /// The kept text before `code` byte offset `at`.
    pub(super) fn kept_before(&self, at: usize) -> &str {
        match self.map.get(at) {
            Some(end) => &self.kept[..*end],
            None => &self.kept,
        }
    }

    pub(super) fn kept(&self) -> &str {
        &self.kept
    }

    fn code(&mut self, ch: char) {
        let offset = self.kept.len();
        self.map.extend(std::iter::repeat_n(offset, ch.len_utf8()));
        self.code.push(ch);
        self.kept.push(ch);
    }

    fn literal(&mut self, ch: char) {
        self.kept.push(ch);
    }

    fn mark(&self) -> (usize, usize) {
        (self.code.len(), self.kept.len())
    }

    fn reset(&mut self, (code, kept): (usize, usize)) {
        self.code.truncate(code);
        self.map.truncate(code);
        self.kept.truncate(kept);
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
    /// Inside a C++ raw string `R"delim( ... )delim"`.
    CppRaw {
        delim: [char; 16],
        len: usize,
    },
}

/// Preprocessor conditionals: only the first branch of each `#if` /
/// `#ifdef` / `#ifndef` is read (none of `#if 0`, whose `#else` is read
/// instead). Branches written as alternatives usually open the same braces,
/// so reading every one of them unbalanced the scan.
#[derive(Debug, Default)]
struct Conditionals {
    /// Per open conditional: (this branch is skipped, a branch was taken).
    frames: Vec<(bool, bool)>,
}

impl Conditionals {
    fn skipping(&self) -> bool {
        self.frames.iter().any(|(skipped, _)| *skipped)
    }

    fn directive(&mut self, line: &str) {
        let rest = line.trim_start().trim_start_matches('#').trim_start();
        let word: String = rest.chars().take_while(char::is_ascii_alphabetic).collect();
        let argument = rest[word.len()..].trim();
        match word.as_str() {
            "if" | "ifdef" | "ifndef" => {
                let never = word == "if" && matches!(argument, "0" | "false");
                self.frames.push((never, !never));
            }
            "elif" | "elifdef" | "elifndef" | "else" => {
                if let Some((skipped, taken)) = self.frames.last_mut() {
                    *skipped = *taken;
                    *taken = true;
                }
            }
            "endif" => {
                self.frames.pop();
            }
            _ => {}
        }
    }
}

/// `text`'s lines (as [`str::lines`] splits them), read as code.
pub(super) fn code_lines(text: &str, syntax: Syntax) -> Vec<CodeLine> {
    let mut state = State::Code;
    let mut conditionals = Conditionals::default();
    let directives = matches!(
        syntax,
        Syntax::CFamily {
            preprocessor: true,
            ..
        }
    );
    text.lines()
        .map(|line| {
            if directives && state == State::Code && line.trim_start().starts_with('#') {
                conditionals.directive(line);
                return CodeLine::default();
            }
            if conditionals.skipping() {
                return CodeLine::default();
            }
            code_line(line, syntax, &mut state)
        })
        .collect()
}

fn code_line(line: &str, syntax: Syntax, state: &mut State) -> CodeLine {
    let mut out = CodeLine::default();
    let chars: Vec<char> = line.chars().collect();
    let mut opened: Option<(usize, (usize, usize))> = None;
    let mut at = 0;
    loop {
        while at < chars.len() {
            let (from, mark) = (at, out.mark());
            let was_code = *state == State::Code;
            at = step(&chars, at, syntax, state, &mut out);
            if was_code && single_line_quote(*state) {
                opened = Some((from, mark));
            }
        }
        let Some((opener, mark)) = opened.take().filter(|_| single_line_quote(*state)) else {
            return out;
        };
        out.reset(mark);
        out.code(chars[opener]);
        *state = State::Code;
        at = opener + 1;
    }
}

fn single_line_quote(state: State) -> bool {
    matches!(
        state,
        State::Quoted {
            multiline: false,
            ..
        }
    )
}

fn step(chars: &[char], at: usize, syntax: Syntax, state: &mut State, out: &mut CodeLine) -> usize {
    match *state {
        State::Code => code_char(chars, at, syntax, state, out),
        State::Block(depth) => block_char(chars, at, depth, syntax, state),
        State::Quoted {
            close,
            width,
            escapes,
            ..
        } => quoted_char(chars, at, (close, width, escapes), state, out),
        State::Raw(hashes) => raw_char(chars, at, hashes, state, out),
        State::CppRaw { delim, len } => cpp_raw_char(chars, at, &delim[..len], state, out),
    }
}

fn starts(chars: &[char], at: usize, text: &str) -> bool {
    text.chars()
        .enumerate()
        .all(|(offset, ch)| chars.get(at + offset) == Some(&ch))
}

fn ident_char(ch: Option<&char>) -> bool {
    ch.is_some_and(|ch| ch.is_alphanumeric() || *ch == '_')
}

fn hash_comment(chars: &[char], at: usize, syntax: Syntax) -> bool {
    if chars[at] != '#' {
        return false;
    }
    match syntax {
        Syntax::Python => true,
        Syntax::Shell => at == 0 || matches!(chars[at - 1], ' ' | '\t' | ';' | '|' | '&' | '('),
        _ => false,
    }
}

/// One step in code; returns the next index.
fn code_char(
    chars: &[char],
    at: usize,
    syntax: Syntax,
    state: &mut State,
    out: &mut CodeLine,
) -> usize {
    let slashes = matches!(syntax, Syntax::Rust | Syntax::CFamily { .. });
    if hash_comment(chars, at, syntax) || (slashes && starts(chars, at, "//")) {
        return chars.len();
    }
    if slashes && starts(chars, at, "/*") {
        out.code(' ');
        *state = State::Block(1);
        return at + 2;
    }
    if syntax == Syntax::Rust
        && let Some(next) = raw_string_open(chars, at, state)
    {
        out.code('"');
        return next;
    }
    let cpp = matches!(
        syntax,
        Syntax::CFamily {
            preprocessor: true,
            ..
        }
    );
    if cpp && let Some(next) = cpp_raw_string_open(chars, at, state) {
        out.code('"');
        return next;
    }
    match chars[at] {
        '"' | '\'' => open_quote(chars, at, syntax, state, out),
        '`' if matches!(syntax, Syntax::CFamily { .. }) => {
            open_quote(chars, at, syntax, state, out)
        }
        ch => {
            out.code(ch);
            at + 1
        }
    }
}

/// C++ `R"delim(` (also `u8R`, `uR`, `UR`, `LR`) at `at`, not inside an
/// identifier.
fn cpp_raw_string_open(chars: &[char], at: usize, state: &mut State) -> Option<usize> {
    if chars[at] != 'R' || chars.get(at + 1) != Some(&'"') {
        return None;
    }
    let prefix: String = chars[..at]
        .iter()
        .rev()
        .take_while(|ch| ident_char(Some(ch)))
        .collect();
    if !matches!(prefix.as_str(), "" | "8u" | "u" | "U" | "L") {
        return None;
    }
    let open = (at + 2..chars.len().min(at + 19)).find(|index| chars[*index] == '(')?;
    let mut delim = ['\0'; 16];
    let len = open - (at + 2);
    if len > 16
        || chars[at + 2..open]
            .iter()
            .any(|ch| ch.is_whitespace() || *ch == '\\')
    {
        return None;
    }
    delim[..len].copy_from_slice(&chars[at + 2..open]);
    *state = State::CppRaw { delim, len };
    Some(open + 1)
}

fn cpp_raw_char(
    chars: &[char],
    at: usize,
    delim: &[char],
    state: &mut State,
    out: &mut CodeLine,
) -> usize {
    let closes = chars[at] == ')'
        && chars.get(at + 1..at + 1 + delim.len()) == Some(delim)
        && chars.get(at + 1 + delim.len()) == Some(&'"');
    if !closes {
        out.literal(chars[at]);
        return at + 1;
    }
    out.code('"');
    *state = State::Code;
    at + 2 + delim.len()
}

/// `r"`, `r#"`, `br##"`, `cr#"` ... at `at`, not inside an identifier.
fn raw_string_open(chars: &[char], at: usize, state: &mut State) -> Option<usize> {
    let r = if matches!(chars[at], 'b' | 'c') {
        at + 1
    } else {
        at
    };
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
    out: &mut CodeLine,
) -> usize {
    let ch = chars[at];
    if syntax == Syntax::Rust && ch == '\'' {
        return rust_quote(chars, at, out);
    }
    let triple = matches!(syntax, Syntax::CFamily { .. } | Syntax::Python)
        && ch != '`'
        && starts(chars, at, &ch.to_string().repeat(3));
    let width = if triple { 3 } else { 1 };
    let raw_backtick = matches!(
        syntax,
        Syntax::CFamily {
            raw_backticks: true,
            ..
        }
    ) && ch == '`';
    let spans_lines = match syntax {
        Syntax::Rust => true,
        Syntax::Shell => ch == '"',
        _ => triple || ch == '`',
    };
    *state = State::Quoted {
        close: ch,
        width,
        escapes: !raw_backtick && !(syntax == Syntax::Shell && ch == '\''),
        multiline: spans_lines,
    };
    for _ in 0..width {
        out.code(ch);
    }
    at + width
}

/// A Rust `'`: a character literal (`'x'`, `'\n'`, `'\''`) becomes `''`; a
/// lifetime or label (`'a`, `'outer:`) is kept as code.
fn rust_quote(chars: &[char], at: usize, out: &mut CodeLine) -> usize {
    let end = if chars.get(at + 1) == Some(&'\\') {
        (at + 3..chars.len()).find(|index| chars[*index] == '\'')
    } else if chars.get(at + 2) == Some(&'\'') {
        Some(at + 2)
    } else {
        None
    };
    let Some(end) = end else {
        out.code('\'');
        return at + 1;
    };
    out.code('\'');
    chars[at + 1..end].iter().for_each(|ch| out.literal(*ch));
    out.code('\'');
    end + 1
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
    out: &mut CodeLine,
) -> usize {
    if escapes && chars[at] == '\\' {
        chars[at..].iter().take(2).for_each(|ch| out.literal(*ch));
        return at + 2;
    }
    if starts(chars, at, &close.to_string().repeat(width)) {
        for _ in 0..width {
            out.code(close);
        }
        *state = State::Code;
        return at + width;
    }
    out.literal(chars[at]);
    at + 1
}

fn raw_char(
    chars: &[char],
    at: usize,
    hashes: usize,
    state: &mut State,
    out: &mut CodeLine,
) -> usize {
    let closes =
        chars[at] == '"' && (1..=hashes).all(|offset| chars.get(at + offset) == Some(&'#'));
    if !closes {
        out.literal(chars[at]);
        return at + 1;
    }
    out.code('"');
    *state = State::Code;
    at + 1 + hashes
}
