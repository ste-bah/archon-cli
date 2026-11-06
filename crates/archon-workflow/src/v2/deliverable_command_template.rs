//! Closed whitelist for the expansion-only command-template domain.
//!
//! Each byte is consumed once. Frames carry their result to their parent;
//! substitutions and default operands never rescan a suffix or recurse.

const MAX_BYTES: usize = 4096;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Copy, Default)]
enum Assignment {
    #[default]
    Start,
    Name,
    Plus,
    Value,
    No,
}

#[derive(Default)]
struct Word {
    present: bool,
    literal: bool,
    assignment: Assignment,
    digits: bool,
    start: usize,
}

impl Word {
    fn add(&mut self, at: usize, byte: Option<u8>, empty: bool) {
        if !self.present {
            self.start = at;
            self.digits = true;
        }
        self.present = true;
        self.literal |= !empty;
        self.digits &= byte.is_some_and(|c| c.is_ascii_digit());
        self.assignment = match (self.assignment, byte) {
            (Assignment::Start, Some(c)) if name_start(c) => Assignment::Name,
            (Assignment::Name, Some(c)) if name_byte(c) => Assignment::Name,
            (Assignment::Name, Some(b'+')) => Assignment::Plus,
            (Assignment::Name | Assignment::Plus, Some(b'=')) => Assignment::Value,
            (Assignment::Value, _) => Assignment::Value,
            _ => Assignment::No,
        };
    }
}

struct List {
    template: bool,
    leading: bool,
    target: bool,
    word: Word,
}

impl Default for List {
    fn default() -> Self {
        Self {
            template: true,
            leading: true,
            target: false,
            word: Word::default(),
        }
    }
}

impl List {
    fn finish_word(&mut self, input: &[u8], end: usize, redirection: bool) {
        if !self.word.present {
            return;
        }
        let word = std::mem::take(&mut self.word);
        if self.target {
            self.target = false;
        } else if !(redirection && word.digits)
            && !matches!(&input[word.start..end], b"{" | b"}" | b"(" | b")")
            && !(self.leading && matches!(word.assignment, Assignment::Value))
        {
            // A bare `!` is a negation token, not ignorable grouping:
            // even a negated empty command fails and is never a template.
            self.leading = false;
            self.template &= !word.literal;
        }
    }

    fn separator(&mut self, input: &[u8], at: usize) {
        self.finish_word(input, at, false);
        self.leading = true;
        self.target = false;
    }
}

#[derive(Clone, Copy)]
enum Parameter {
    Unknown,
    Default,
    Guaranteed,
}

#[derive(Clone, Copy)]
enum Kind {
    List(u8), // 0 = root, ')' = command substitution/group, '`' = backticks
    Single,
    Double,
    Parameter(Parameter),
    Arithmetic,
    ArithmeticGroup,
}

struct Frame {
    kind: Kind,
    list: List,
    empty: bool,
    start: usize,
    // Grouping has list semantics, but contributes no word to its parent.
    group: bool,
}

impl Frame {
    fn new(kind: Kind, start: usize, group: bool) -> Self {
        Self {
            kind,
            list: List::default(),
            empty: true,
            start,
            group,
        }
    }

    fn add(&mut self, at: usize, byte: Option<u8>, empty: bool) {
        if matches!(self.kind, Kind::List(_)) {
            self.list.word.add(at, byte, empty);
        } else {
            self.empty &= empty;
        }
    }
}

fn name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn name_byte(c: u8) -> bool {
    name_start(c) || c.is_ascii_digit()
}

/// The only parameter forms that can guarantee a nonempty word. Header bytes
/// are consumed here, once; operand bytes are consumed by the main scanner.
fn parameter(input: &[u8], at: &mut usize) -> Parameter {
    let length = input.get(*at) == Some(&b'#');
    if length {
        *at += 1;
    }
    if !input.get(*at).is_some_and(|c| name_start(*c)) {
        return Parameter::Unknown;
    }
    *at += 1;
    while input.get(*at).is_some_and(|c| name_byte(*c)) {
        *at += 1;
    }
    if length {
        return if input.get(*at) == Some(&b'}') {
            Parameter::Guaranteed
        } else {
            Parameter::Unknown
        };
    }
    if input.get(*at) == Some(&b':') {
        *at += 1;
    }
    match input.get(*at) {
        Some(b'-' | b'=') => {
            *at += 1;
            Parameter::Default
        }
        Some(b'?') => {
            *at += 1;
            Parameter::Guaranteed
        }
        _ => Parameter::Unknown,
    }
}

pub(super) fn expansion_only(value: &str) -> bool {
    if value.len() > MAX_BYTES {
        return false;
    }
    let input = value.as_bytes();
    let mut stack = vec![Frame::new(Kind::List(0), 0, false)];
    let mut at = 0;
    while at < input.len() {
        let c = input[at];
        let frame = stack.last_mut().unwrap();
        let mut close = 0;
        match frame.kind {
            Kind::Single => {
                if c == b'\'' {
                    close = 1;
                } else {
                    at += 1;
                    continue;
                }
            }
            Kind::Double if c == b'"' => close = 1,
            Kind::Parameter(_) if c == b'}' => close = 1,
            Kind::List(end) if end != 0 && c == end => close = 1,
            Kind::Arithmetic if input[at..].starts_with(b"))") => close = 2,
            Kind::ArithmeticGroup if c == b')' => close = 1,
            _ => {}
        }
        if close != 0 {
            let mut done = stack.pop().unwrap();
            let empty = match done.kind {
                Kind::List(_) => {
                    done.list.finish_word(input, at, false);
                    done.list.template
                }
                Kind::Single | Kind::Arithmetic | Kind::ArithmeticGroup => false,
                Kind::Parameter(Parameter::Guaranteed) => false,
                Kind::Parameter(Parameter::Unknown) => true,
                _ => done.empty,
            };
            let parent = stack.last_mut().unwrap();
            if done.group {
                parent.list.template &= empty;
                parent.list.leading = true;
            } else {
                parent.add(done.start, None, empty);
            }
            at += close;
            continue;
        }
        let list = matches!(frame.kind, Kind::List(_));
        let arithmetic = matches!(frame.kind, Kind::Arithmetic | Kind::ArithmeticGroup);
        if list {
            if c == b'#' && !frame.list.word.present {
                while at < input.len() && input[at] != b'\n' {
                    at += 1;
                }
                continue;
            }
            if c == b'<' || c == b'>' || (c == b'&' && input.get(at + 1) == Some(&b'>')) {
                frame.list.finish_word(input, at, true);
                frame.list.target = true;
                let first = c;
                at += 1;
                if first == b'&' {
                    at += 1;
                    if input.get(at) == Some(&b'>') {
                        at += 1;
                    }
                } else if input.get(at) == Some(&first) {
                    at += 1;
                    if first == b'<' && matches!(input.get(at), Some(b'<' | b'-')) {
                        at += 1;
                    }
                } else if matches!(input.get(at), Some(b'&' | b'|')) {
                    at += 1;
                }
                continue;
            }
            if matches!(c, b';' | b'&' | b'|' | b'\n') {
                frame.list.separator(input, at);
                at += 1;
                continue;
            }
            if c.is_ascii_whitespace() {
                frame.list.finish_word(input, at, false);
                at += 1;
                continue;
            }
        }
        let start = at;
        let kind = if c == b'\'' && !matches!(frame.kind, Kind::Double) {
            at += 1;
            Some((Kind::Single, false))
        } else if c == b'"' {
            at += 1;
            Some((Kind::Double, false))
        } else if c == b'`' {
            at += 1;
            Some((Kind::List(b'`'), false))
        } else if input[at..].starts_with(b"$((") {
            at += 3;
            Some((Kind::Arithmetic, false))
        } else if input[at..].starts_with(b"$(") {
            at += 2;
            Some((Kind::List(b')'), false))
        } else if input[at..].starts_with(b"${") {
            at += 2;
            Some((Kind::Parameter(parameter(input, &mut at)), false))
        } else if c == b'(' && (list || arithmetic) {
            if list {
                frame.list.separator(input, at);
            }
            at += 1;
            Some((
                if list {
                    Kind::List(b')')
                } else {
                    Kind::ArithmeticGroup
                },
                list,
            ))
        } else {
            None
        };
        if let Some((kind, group)) = kind {
            if stack.len() > MAX_DEPTH {
                return false;
            }
            stack.push(Frame::new(kind, start, group));
            continue;
        }
        if c == b'\\' {
            if at + 1 == input.len() {
                return false;
            }
            // An escaped newline contributes no character; other escapes do.
            if input[at + 1] != b'\n' {
                frame.add(at, None, false);
            }
            at += 2;
        } else if c == b'$'
            && input
                .get(at + 1)
                .is_some_and(|c| name_byte(*c) || b"@*!#?$-".contains(c))
        {
            frame.add(at, None, true);
            at += 1;
            if name_start(input[at]) {
                at += 1;
                while input.get(at).is_some_and(|c| name_byte(*c)) {
                    at += 1;
                }
            } else {
                at += 1;
            }
        } else if list && c == b')' {
            // Unmatched grouping is malformed, and belongs to the syntax gate.
            return false;
        } else {
            frame.add(at, Some(c), false);
            at += 1;
        }
    }
    if stack.len() != 1 {
        return false;
    }
    let root = &mut stack[0];
    root.list.finish_word(input, at, false);
    root.list.template
}

#[cfg(test)]
mod tests {
    use super::expansion_only;

    #[test]
    fn caps_and_unterminated_delimiters_are_not_templates() {
        let hostile = [
            "$(".repeat(10_000),
            "${".repeat(10_000),
            format!("{}$X{}", "$(".repeat(5_000), ")".repeat(5_000)),
            " ".repeat(4097),
            format!("{}$X{}", "$(".repeat(33), ")".repeat(33)),
            format!("{}$X{}", "${X:-".repeat(33), "}".repeat(33)),
            "'unterminated".to_string(),
            "\"unterminated".to_string(),
            "`unterminated".to_string(),
            "${X".to_string(),
            "$(true".to_string(),
            "$((1+1)".to_string(),
            "\\".to_string(),
        ];
        let start = std::time::Instant::now();
        for _ in 0..64 {
            for input in &hostile {
                assert!(!expansion_only(input), "{input:.80}");
            }
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        assert!(expansion_only(&"$X".repeat(2048)));
        assert!(expansion_only(&format!(
            "{}$X{}",
            "$(".repeat(32),
            ")".repeat(32)
        )));
    }

    #[test]
    fn closed_whitelist_and_token_boundaries() {
        for input in [
            "$#",
            "$?",
            "$$",
            "$-",
            "${X:=}",
            "${X=}",
            "${X:unknown}",
            "${X:-\"$Y\"}",
            "$X &>/dev/null",
            "$X 3>>out",
            "$X 4<input",
            "$X 2>&1",
            "$X 2>&-",
            "$X >&2",
            "$X <<<data",
            "$X <<EOF",
            "$X <<- EOF",
            "$X # ignored literal\n$Y",
            "A+=1 $X",
            "a=$(echo x) $X",
            "${X:-$(true)}",
            "${X:-`$Y`}",
        ] {
            assert!(expansion_only(input), "{input}");
        }
        for input in [
            "! $X",
            "! ${X}",
            "'$X'",
            "'${X}'",
            "''",
            "$X#literal",
            "$X \\#literal",
            "X=1 literal",
            "$X X=1",
            "$X >out literal",
            "$X && echo ok",
            "${X:=literal}",
            "${X=literal}",
            "${X-literal}",
            "${X:?}",
            "${X?}",
            "${#NAME}",
            "$((1+(2)))",
            "${X:-$(echo x)}",
            "${X:-'$Y'}",
            "${X:-${Y:-z}}",
            "$(echo '$X; $Y')",
            "${1:-literal} literal",
            "$X \\",
        ] {
            assert!(!expansion_only(input), "{input}");
        }
    }
}
