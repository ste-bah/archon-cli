//! The command words each statement runs, so a caller can tell whether an
//! earlier statement can end the script (`exit`, `kill $$`, `set -e`, …).

use super::{Ctx, Lexer, Statement};

/// An unquoted word in command position.
#[derive(Debug, Clone, Default)]
pub(in crate::verifier_strength) struct Command {
    pub(in crate::verifier_strength) name: String,
    /// Raw text after the name, up to the next `;`, `&`, `|`, `)` or newline.
    pub(in crate::verifier_strength) args: String,
    /// The function whose body contains the command, if any.
    pub(in crate::verifier_strength) function: Option<String>,
    /// Inside `$( )`, `( )` or backticks, where `exit` ends only a subshell.
    pub(in crate::verifier_strength) subshell: bool,
    offset: usize,
}

impl Lexer {
    pub(super) fn record_command(&mut self, name: &str) {
        while self
            .function_frames
            .last()
            .is_some_and(|(depth, _)| *depth > self.stack.len())
        {
            self.function_frames.pop();
        }
        let function = self.function_frames.last().map(|(_, name)| name.clone());
        let subshell = self
            .stack
            .iter()
            .any(|(ctx, _)| matches!(ctx, Ctx::Paren | Ctx::Backtick));
        self.statement.commands.push(Command {
            name: name.to_string(),
            args: String::new(),
            function,
            subshell,
            offset: self.statement.text.len(),
        });
    }

    /// `name ()` or `function name`: the next frame pushed is its body. The
    /// name itself was recorded as a command word, but it is not a call.
    pub(super) fn begin_function(&mut self, name: String) {
        if self
            .statement
            .commands
            .last()
            .is_some_and(|command| command.name == name)
        {
            self.statement.commands.pop();
        }
        self.pending_function = Some(name);
    }

    /// Called after every push: a pending function's body starts here.
    pub(super) fn frame_pushed(&mut self) {
        if let Some(name) = self.pending_function.take() {
            self.function_frames.push((self.stack.len(), name));
        }
    }
}

/// Fill each command's `args` from the untrimmed statement text.
pub(super) fn fill_args(statement: &mut Statement) {
    for command in &mut statement.commands {
        let rest = &statement.text[command.offset..];
        let end = rest.find([';', '&', '|', '\n', ')']).unwrap_or(rest.len());
        command.args = rest[..end].trim().to_string();
    }
}
