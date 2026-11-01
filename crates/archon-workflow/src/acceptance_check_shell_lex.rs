//! Issue 554: the simple commands of a shell text, as the shell would see
//! them, for the one question the preflight asks: does the text run a
//! command that changes how its later lines parse? Comments, quoted text,
//! here-document bodies and command substitutions run no such command in
//! the text's own shell, so they yield none. Every word comes back with its
//! quoting removed (`\alias`, `'alias'` and `"alias"` all read `alias`).

/// Reserved words after which the next word is again in command position.
const OPENERS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "!", "{", "}", "time",
    "esac",
];

/// The words of every simple command of `text`, its leading assignments
/// and reserved words left out.
pub(super) fn simple_commands(text: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut lexer = Lexer::default();
    let mut at = 0;
    while at < chars.len() {
        at = lexer.step(&chars, at);
    }
    lexer.end_command();
    lexer.commands
}

#[derive(Default)]
struct Lexer {
    commands: Vec<Vec<String>>,
    command: Vec<String>,
    word: String,
    /// Whether a word is open (it may be empty: `''`).
    in_word: bool,
    /// The next word is a redirection target, never a command word.
    redirect: bool,
    /// Here-documents whose bodies start at the next newline: (delimiter,
    /// whether leading tabs are stripped).
    heredocs: Vec<(String, bool)>,
    /// The next word is a here-document delimiter (`-` for `<<-`).
    delimiter: Option<bool>,
}

impl Lexer {
    fn step(&mut self, chars: &[char], at: usize) -> usize {
        let ch = chars[at];
        let next = chars.get(at + 1).copied();
        match ch {
            '#' if !self.in_word => skip_until(chars, at, |c| c == '\n'),
            '\\' => match next {
                Some('\n') => at + 2,
                Some(escaped) => {
                    self.push(escaped);
                    at + 2
                }
                None => at + 1,
            },
            '\'' => {
                self.in_word = true;
                let end = skip_until(chars, at + 1, |c| c == '\'');
                self.word.extend(&chars[at + 1..end.min(chars.len())]);
                end + 1
            }
            '"' => {
                self.in_word = true;
                let mut i = at + 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                    }
                    self.word.push(chars[i]);
                    i += 1;
                }
                i + 1
            }
            '`' => {
                // A command substitution runs in a subshell: nothing it
                // defines changes the parse of this text.
                self.in_word = true;
                skip_until(chars, at + 1, |c| c == '`') + 1
            }
            '$' if next == Some('(') => {
                self.in_word = true;
                skip_balanced(chars, at + 1)
            }
            ' ' | '\t' => {
                self.end_word();
                at + 1
            }
            '\n' => {
                self.end_command();
                self.bodies(chars, at + 1)
            }
            ';' | '&' | '|' | '(' | ')' => {
                self.end_command();
                at + 1
            }
            '<' if next == Some('<') && chars.get(at + 2) != Some(&'<') => {
                self.end_word();
                let strip = chars.get(at + 2) == Some(&'-');
                self.delimiter = Some(strip);
                at + if strip { 3 } else { 2 }
            }
            '<' | '>' => {
                self.end_word();
                self.redirect = true;
                skip_until(chars, at, |c| !matches!(c, '<' | '>' | '&' | '0'..='9'))
            }
            _ => {
                self.push(ch);
                at + 1
            }
        }
    }

    fn push(&mut self, ch: char) {
        self.in_word = true;
        self.word.push(ch);
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        self.in_word = false;
        if let Some(strip) = self.delimiter.take() {
            self.heredocs.push((word, strip));
        } else if std::mem::take(&mut self.redirect) {
        } else if self.command.is_empty() && (OPENERS.contains(&word.as_str()) || assignment(&word))
        {
        } else {
            self.command.push(word);
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        if !self.command.is_empty() {
            self.commands.push(std::mem::take(&mut self.command));
        }
    }

    /// Skip the bodies of the here-documents opened on the line just ended.
    fn bodies(&mut self, chars: &[char], mut at: usize) -> usize {
        for (delimiter, strip) in std::mem::take(&mut self.heredocs) {
            while at < chars.len() {
                let end = skip_until(chars, at, |c| c == '\n');
                let line: String = chars[at..end.min(chars.len())].iter().collect();
                at = end + 1;
                let line = if strip {
                    line.trim_start_matches('\t')
                } else {
                    &line
                };
                if line == delimiter {
                    break;
                }
            }
        }
        at
    }
}

fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn skip_until(chars: &[char], mut at: usize, stop: impl Fn(char) -> bool) -> usize {
    while at < chars.len() && !stop(chars[at]) {
        at += 1;
    }
    at
}

/// Past the `)` matching the `(` at `open`, quotes respected.
fn skip_balanced(chars: &[char], open: usize) -> usize {
    let mut depth = 0usize;
    let mut at = open;
    while at < chars.len() {
        match chars[at] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return at + 1;
                }
            }
            '\'' => at = skip_until(chars, at + 1, |c| c == '\''),
            '"' => at = skip_until(chars, at + 1, |c| c == '"'),
            '\\' => at += 1,
            _ => {}
        }
        at += 1;
    }
    at
}
