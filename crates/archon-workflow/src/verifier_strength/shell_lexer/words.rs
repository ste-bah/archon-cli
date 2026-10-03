//! Quoting, expansions, heredocs and reserved words for [`super::Lexer`].

use super::{Ctx, Lexer, WordState};

impl Lexer {
    /// `'...'` starting at `self.i`; `ansi` (`$'...'`) honours backslash escapes.
    pub(super) fn single_quoted(&mut self, ansi: bool) -> Option<()> {
        let start = self.i;
        let mut j = self.i + if ansi { 2 } else { 1 };
        loop {
            match self.chars.get(j)? {
                '\\' if ansi => j += 2,
                '\'' => break,
                _ => j += 1,
            }
        }
        let text: String = self.chars[start..=j].iter().collect();
        self.i = j + 1;
        self.word_text(&text, true);
        Some(())
    }

    /// `((...))` arithmetic: skipped as one opaque unit.
    pub(super) fn arithmetic(&mut self, from: usize) -> Option<()> {
        let mut depth = 0usize;
        let mut j = from;
        loop {
            match self.chars.get(j)? {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let text: String = self.chars[self.i..=j].iter().collect();
        self.i = j + 1;
        self.word_text(&text, true);
        Some(())
    }

    pub(super) fn dquote_step(&mut self) -> Option<()> {
        let c = self.chars[self.i];
        match (c, self.peek(1)) {
            ('\\', Some(next)) => {
                self.emit(&format!("{c}{next}"));
                self.i += 2;
            }
            ('\\', None) => return None,
            ('"', _) => {
                self.stack.pop();
                self.emit("\"");
                self.i += 1;
            }
            _ => self.expansion_or(c)?,
        }
        Some(())
    }

    pub(super) fn param_step(&mut self) -> Option<()> {
        let c = self.chars[self.i];
        match (c, self.peek(1)) {
            ('\\', Some(next)) => {
                self.emit(&format!("{c}{next}"));
                self.i += 2;
            }
            ('\\', None) => return None,
            ('}', _) => {
                self.stack.pop();
                self.emit("}");
                self.i += 1;
            }
            ('\'', _) => self.single_quoted(false)?,
            ('"', _) => {
                self.stack.push((Ctx::DQuote, self.state));
                self.emit("\"");
                self.i += 1;
            }
            _ => self.expansion_or(c)?,
        }
        Some(())
    }

    /// `$((`, `$(`, `${` and backticks open nested units in any context;
    /// any other character is literal text of the current word.
    pub(super) fn expansion_or(&mut self, c: char) -> Option<()> {
        match (c, self.peek(1), self.peek(2)) {
            ('$', Some('('), Some('(')) => return self.arithmetic(self.i + 1),
            ('$', Some('('), _) => self.open_substitution(Ctx::Paren, "$(", 2),
            ('`', _, _) => self.open_substitution(Ctx::Backtick, "`", 1),
            ('$', Some('{'), _) => {
                self.word_text("${", true);
                self.stack.push((Ctx::Param, self.state));
                self.i += 2;
            }
            _ => {
                let quoted = matches!(self.top(), Some(Ctx::DQuote | Ctx::Param));
                self.word_text(&c.to_string(), quoted);
                self.i += 1;
            }
        }
        Some(())
    }

    pub(super) fn open_substitution(&mut self, ctx: Ctx, text: &str, width: usize) {
        self.word_text(text, true);
        self.stack.push((ctx, self.state));
        // The outer word is quoted now, so its spelling no longer matters;
        // the substitution's own words start from an empty buffer.
        self.word.clear();
        self.state = WordState {
            command_position: true,
            ..WordState::default()
        };
        self.i += width;
    }

    pub(super) fn close_nested(&mut self, text: &str) -> Option<()> {
        self.end_word()?;
        let (_, saved) = self.stack.pop()?;
        self.state = saved;
        if !saved.in_word {
            // A subshell ended; what follows is an operator or redirection.
            self.state.command_position = false;
        }
        self.emit(text);
        self.i += 1;
        Some(())
    }

    pub(super) fn heredoc(&mut self) -> Option<()> {
        self.end_word()?;
        self.i += 2;
        let strip_tabs = self.peek(0) == Some('-');
        if strip_tabs {
            self.i += 1;
        }
        while matches!(self.peek(0), Some(' ' | '\t')) {
            self.i += 1;
        }
        let mut delimiter = String::new();
        let mut quote = None;
        while let Some(c) = self.peek(0) {
            match (quote, c) {
                (None, ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>') => break,
                (None, '\'' | '"') => quote = Some(c),
                (Some(open), _) if open == c => quote = None,
                (None, '\\') => {
                    self.i += 1;
                    delimiter.push(self.peek(0)?);
                }
                _ => delimiter.push(c),
            }
            self.i += 1;
        }
        if quote.is_some() || delimiter.is_empty() {
            return None;
        }
        self.emit(&format!("<<{delimiter}"));
        self.after_operator = false;
        self.heredocs.push((delimiter, strip_tabs));
        Some(())
    }

    pub(super) fn skip_heredoc_bodies(&mut self) -> Option<()> {
        for (delimiter, strip_tabs) in std::mem::take(&mut self.heredocs) {
            loop {
                if self.i >= self.chars.len() {
                    return None;
                }
                let end = (self.i..self.chars.len())
                    .find(|&j| self.chars[j] == '\n')
                    .unwrap_or(self.chars.len());
                let line: String = self.chars[self.i..end].iter().collect();
                self.i = (end + 1).min(self.chars.len());
                let line = line.strip_suffix('\r').unwrap_or(&line);
                let line = if strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line
                };
                if line == delimiter {
                    break;
                }
            }
        }
        Some(())
    }

    pub(super) fn reserved_word(&mut self, word: &str) -> Option<()> {
        let expect = |lexer: &Self, ctx: Ctx| (lexer.top() == Some(ctx)).then_some(());
        match word {
            "if" => self.push(Ctx::If, true),
            "while" | "until" => self.push(Ctx::Loop, true),
            "for" | "select" => {
                self.push(Ctx::Loop, true);
                return Some(());
            }
            "case" => {
                self.push(Ctx::Case, true);
                return Some(());
            }
            "{" => self.push(Ctx::Brace, true),
            "[[" => {
                self.push(Ctx::Test, true);
                return Some(());
            }
            "then" | "elif" | "else" => expect(self, Ctx::If)?,
            "do" => expect(self, Ctx::Loop)?,
            "fi" | "done" | "esac" | "}" => {
                let ctx = match word {
                    "fi" => Ctx::If,
                    "done" => Ctx::Loop,
                    "esac" => Ctx::Case,
                    _ => Ctx::Brace,
                };
                expect(self, ctx)?;
                self.stack.pop();
                return Some(());
            }
            "function" => {
                if self.stack.is_empty() {
                    self.stage.compound = true;
                }
                self.function_name_next = true;
                return Some(());
            }
            "!" | "time" => {}
            _ => return Some(()),
        }
        self.state.command_position = true;
        Some(())
    }
}
