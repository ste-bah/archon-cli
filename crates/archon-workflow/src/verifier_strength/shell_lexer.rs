//! Top-level structure of a POSIX shell script, for verifier classification.
//!
//! The verifier-strength rules judge only the command whose exit status the
//! script returns, so they need the script's top-level statements: newline,
//! `;` and `&` separate statements; `&&` and `||` join pipelines into a list;
//! `|` joins stages into a pipeline. Everything nested — `$(...)`, backticks,
//! `( )`, `{ }`, `[[ ]]`, `(( ))`, `if/while/until/for/select/case` blocks and
//! function bodies — stays inside the stage that contains it, and heredoc
//! bodies and comments are dropped.
//!
//! [`statements`] returns `None` whenever the structure is not certain (an
//! unbalanced quote or block, an unterminated heredoc, a dangling operator).
//! Callers treat `None` as "undecidable", never as a defect.

mod commands;
#[cfg(test)]
mod tests;
mod words;

pub(super) use commands::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListOp {
    And,
    Or,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Stage {
    pub text: String,
    /// The stage contains a top-level compound command or function definition.
    pub compound: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Item {
    /// The list operator that precedes this pipeline (`None` for the first).
    pub op: Option<ListOp>,
    pub stages: Vec<Stage>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Statement {
    pub text: String,
    pub items: Vec<Item>,
    /// The statement was terminated by `&`, so its status is always zero.
    pub background: bool,
    /// Unquoted command words, nested ones included, in source order.
    pub commands: Vec<Command>,
}

/// Split `script` into its top-level statements, or `None` if uncertain.
pub(super) fn statements(script: &str) -> Option<Vec<Statement>> {
    let mut lexer = Lexer::new(script);
    lexer.run()?;
    Some(lexer.out)
}

#[derive(Debug, Clone, Copy, Default)]
struct WordState {
    in_word: bool,
    quoted: bool,
    at_command: bool,
    command_position: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// `$(`, a subshell `(`, or a process substitution.
    Paren,
    Backtick,
    DQuote,
    Param,
    Brace,
    Test,
    If,
    Loop,
    Case,
}

struct Lexer {
    chars: Vec<char>,
    i: usize,
    stack: Vec<(Ctx, WordState)>,
    word: String,
    state: WordState,
    after_operator: bool,
    function_name_next: bool,
    last_word: String,
    pending_function: Option<String>,
    /// `(stack depth of the body frame, function name)` for open bodies.
    function_frames: Vec<(usize, String)>,
    heredocs: Vec<(String, bool)>,
    out: Vec<Statement>,
    statement: Statement,
    item: Item,
    stage: Stage,
}

impl Lexer {
    fn new(script: &str) -> Self {
        Self {
            chars: script.chars().collect(),
            i: 0,
            stack: Vec::new(),
            word: String::new(),
            state: WordState {
                command_position: true,
                ..WordState::default()
            },
            after_operator: false,
            function_name_next: false,
            last_word: String::new(),
            pending_function: None,
            function_frames: Vec::new(),
            heredocs: Vec::new(),
            out: Vec::new(),
            statement: Statement::default(),
            item: Item::default(),
            stage: Stage::default(),
        }
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.i + offset).copied()
    }

    fn top(&self) -> Option<Ctx> {
        self.stack.last().map(|(ctx, _)| *ctx)
    }

    fn emit(&mut self, text: &str) {
        self.stage.text.push_str(text);
        self.statement.text.push_str(text);
    }

    /// Emit text that belongs to the current word (quoted text never forms a
    /// reserved word, but it does start a command name).
    fn word_text(&mut self, text: &str, quoted: bool) {
        if !self.state.in_word {
            self.state.in_word = true;
            self.state.quoted = false;
            self.state.at_command = self.state.command_position;
        }
        self.state.quoted |= quoted;
        if !quoted {
            self.word.push_str(text);
        }
        self.after_operator = false;
        self.emit(text);
    }

    fn push(&mut self, ctx: Ctx, compound: bool) {
        if compound && self.stack.is_empty() {
            self.stage.compound = true;
        }
        self.stack.push((ctx, self.state));
        self.frame_pushed();
    }

    fn run(&mut self) -> Option<()> {
        while self.i < self.chars.len() {
            match self.top() {
                Some(Ctx::DQuote) => self.dquote_step()?,
                Some(Ctx::Param) => self.param_step()?,
                _ => self.script_step()?,
            }
        }
        self.end_word()?;
        if !self.stack.is_empty() || !self.heredocs.is_empty() || self.after_operator {
            return None;
        }
        self.finish_statement(false)
    }

    fn script_step(&mut self) -> Option<()> {
        let c = self.chars[self.i];
        let next = self.peek(1);
        if self.top() == Some(Ctx::Test) {
            let closes = !self.state.in_word
                && c == ']'
                && next == Some(']')
                && self
                    .peek(2)
                    .is_none_or(|after| " \t\r\n;&|)<>".contains(after));
            if closes {
                self.stack.pop();
                self.emit("]]");
                self.state.command_position = false;
                self.i += 2;
                return Some(());
            }
            if !" \t\r\n\\'\"$`".contains(c) {
                self.word_text(&c.to_string(), false);
                self.i += 1;
                return Some(());
            }
        }
        match c {
            '\\' => match next {
                Some('\n') => self.i += 2,
                Some(escaped) => {
                    self.word_text(&format!("\\{escaped}"), true);
                    self.i += 2;
                }
                None => return None,
            },
            '\'' => self.single_quoted(false)?,
            '$' if next == Some('\'') => self.single_quoted(true)?,
            '"' => {
                self.word_text("\"", true);
                self.stack.push((Ctx::DQuote, self.state));
                self.i += 1;
            }
            '`' => {
                if self.state.in_word && self.state.at_command && !self.state.quoted {
                    // A pending `fi`/`done`/`}` must close its block first.
                    self.end_word()?;
                }
                if self.top() == Some(Ctx::Backtick) {
                    self.close_nested("`")?;
                } else {
                    self.open_substitution(Ctx::Backtick, "`", 1);
                }
            }
            '#' if !self.state.in_word => {
                while self.peek(0).is_some_and(|ch| ch != '\n') {
                    self.i += 1;
                }
            }
            ' ' | '\t' | '\r' => {
                self.end_word()?;
                self.emit(" ");
                self.i += 1;
            }
            '\n' => self.newline()?,
            ';' => self.semicolon()?,
            '&' => self.ampersand()?,
            '|' => self.pipe()?,
            '(' => self.open_paren()?,
            ')' => self.close_paren()?,
            '<' if next == Some('<') && self.peek(2) == Some('<') => {
                self.end_word()?;
                self.emit("<<<");
                self.after_operator = false;
                self.i += 3;
            }
            '<' if next == Some('<') => self.heredoc()?,
            '<' | '>' => {
                self.end_word()?;
                self.emit(&c.to_string());
                self.after_operator = false;
                self.i += 1;
            }
            _ => self.expansion_or(c)?,
        }
        Some(())
    }

    fn newline(&mut self) -> Option<()> {
        self.end_word()?;
        self.i += 1;
        if self.after_operator {
            self.emit(" ");
        } else if self.stack.is_empty() {
            self.finish_statement(false)?;
        } else {
            self.emit("\n");
            self.state.command_position = true;
        }
        self.skip_heredoc_bodies()
    }

    fn semicolon(&mut self) -> Option<()> {
        self.end_word()?;
        if self.peek(1) == Some(';') {
            if self.top() != Some(Ctx::Case) {
                return None;
            }
            self.i += if self.peek(2) == Some('&') { 3 } else { 2 };
            self.emit(";;");
            self.state.command_position = true;
            return Some(());
        }
        self.i += if self.peek(1) == Some('&') && self.top() == Some(Ctx::Case) {
            2
        } else {
            1
        };
        self.separator(false)
    }

    fn separator(&mut self, background: bool) -> Option<()> {
        if self.after_operator {
            return None;
        }
        if self.stack.is_empty() {
            self.finish_statement(background)
        } else {
            self.emit(if background { "&" } else { ";" });
            self.state.command_position = true;
            Some(())
        }
    }

    fn ampersand(&mut self) -> Option<()> {
        let prev = self.i.checked_sub(1).map(|j| self.chars[j]);
        if self.peek(1) == Some('&') {
            return self.list_operator(ListOp::And);
        }
        if matches!(prev, Some('>' | '<')) || self.peek(1) == Some('>') {
            self.emit("&");
            self.i += 1;
            return Some(());
        }
        self.end_word()?;
        self.i += 1;
        self.separator(true)
    }

    fn pipe(&mut self) -> Option<()> {
        if self.peek(1) == Some('|') {
            return self.list_operator(ListOp::Or);
        }
        if self.i > 0 && self.chars[self.i - 1] == '>' {
            self.emit("|");
            self.i += 1;
            return Some(());
        }
        self.end_word()?;
        self.i += if self.peek(1) == Some('&') { 2 } else { 1 };
        if self.stack.is_empty() {
            self.finish_stage()?;
            self.statement.text.push_str(" | ");
        } else {
            self.emit("|");
        }
        self.state.command_position = true;
        self.after_operator = self.top() != Some(Ctx::Case);
        Some(())
    }

    fn list_operator(&mut self, op: ListOp) -> Option<()> {
        self.end_word()?;
        self.i += 2;
        let text = if op == ListOp::And { "&&" } else { "||" };
        if self.stack.is_empty() {
            self.finish_stage()?;
            let item = std::mem::take(&mut self.item);
            self.statement.items.push(item);
            self.item.op = Some(op);
            self.statement.text.push_str(&format!(" {text} "));
        } else {
            self.emit(text);
        }
        self.state.command_position = true;
        self.after_operator = true;
        Some(())
    }

    fn open_paren(&mut self) -> Option<()> {
        if self.peek(1) == Some('(') {
            if !self.state.in_word && self.stack.is_empty() {
                self.stage.compound = true;
            }
            return self.arithmetic(self.i);
        }
        if self.top() == Some(Ctx::Case) && self.state.command_position && !self.state.in_word {
            // The optional `(` that opens a case pattern, closed by its `)`.
            self.emit("(");
            self.i += 1;
            return Some(());
        }
        let prev = self.i.checked_sub(1).map(|j| self.chars[j]);
        // `<(`/`>(` process substitution, `name=(` arrays and extglob
        // `@(`-style patterns are words, not subshells.
        let substitution = matches!(prev, Some('<' | '>'))
            || (self.state.in_word && matches!(prev, Some('=' | '@' | '?' | '*' | '+' | '!')));
        self.end_word()?;
        let close = (self.i + 1..self.chars.len()).find(|&j| !matches!(self.chars[j], ' ' | '\t'));
        if !substitution && close.is_some_and(|j| self.chars[j] == ')') {
            // `name ()` — a function definition; its body is the next command.
            if self.stack.is_empty() {
                self.stage.compound = true;
            }
            let name = std::mem::take(&mut self.last_word);
            self.begin_function(name);
            self.emit("()");
            self.i = close? + 1;
            self.state.command_position = true;
            return Some(());
        }
        self.push(Ctx::Paren, !substitution);
        self.emit("(");
        self.after_operator = false;
        self.state = WordState {
            command_position: true,
            ..WordState::default()
        };
        self.i += 1;
        Some(())
    }

    fn close_paren(&mut self) -> Option<()> {
        self.end_word()?;
        match self.top() {
            Some(Ctx::Paren) => self.close_nested(")"),
            Some(Ctx::Case) => {
                // The `)` that ends a case pattern; the arm's command follows.
                self.end_word()?;
                self.emit(")");
                self.state.command_position = true;
                self.after_operator = false;
                self.i += 1;
                Some(())
            }
            _ => None,
        }
    }

    fn end_word(&mut self) -> Option<()> {
        if !self.state.in_word {
            return Some(());
        }
        let word = std::mem::take(&mut self.word);
        let state = self.state;
        self.state.in_word = false;
        if self.function_name_next {
            self.function_name_next = false;
            self.state.command_position = true;
            self.begin_function(word);
            return Some(());
        }
        if !state.at_command {
            return Some(());
        }
        self.state.command_position = false;
        if state.quoted {
            return Some(());
        }
        self.last_word.clone_from(&word);
        self.reserved_word(&word)
    }

    fn finish_stage(&mut self) -> Option<()> {
        let mut stage = std::mem::take(&mut self.stage);
        stage.text = stage.text.trim().to_string();
        if stage.text.is_empty() {
            return None;
        }
        self.item.stages.push(stage);
        Some(())
    }

    fn finish_statement(&mut self, background: bool) -> Option<()> {
        if self.after_operator {
            return None;
        }
        let empty = self.stage.text.trim().is_empty();
        if empty && (self.item.op.is_some() || !self.item.stages.is_empty()) {
            return None;
        }
        if !empty {
            self.finish_stage()?;
            let item = std::mem::take(&mut self.item);
            self.statement.items.push(item);
        }
        let mut statement = std::mem::take(&mut self.statement);
        commands::fill_args(&mut statement);
        self.pending_function = None;
        self.stage = Stage::default();
        self.item = Item::default();
        self.state.command_position = true;
        if !statement.items.is_empty() {
            statement.text = statement.text.trim().to_string();
            statement.background = background;
            self.out.push(statement);
        } else if background {
            return None;
        }
        Some(())
    }
}
