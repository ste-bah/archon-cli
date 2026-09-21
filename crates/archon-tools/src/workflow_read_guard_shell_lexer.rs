//! The read guard's shell lexer: executable segments as word lists, with
//! quotes and comments honoured and heredoc bodies skipped as data.

use std::iter::Peekable;
use std::str::Chars;

/// Lex executable segments separately: a variable in a later echo must not
/// hide an earlier inspection/build. Shell expansion is not evaluated here.
/// A heredoc's operator and delimiter stay in the word list; its body lines
/// are consumed verbatim and never become words or commands (Issue-67).
pub(in crate::workflow_read_guard) fn commands(text: &str) -> Vec<Vec<String>> {
    let mut lexer = Lexer::default();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if comment && ch != '\n' {
            continue;
        }
        if ch == '\n' {
            comment = false;
        }
        if escaped {
            lexer.word.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                lexer.word.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '#' if lexer.word.is_empty() => comment = true,
            '<' | '>' => lexer.redirect(ch, &mut chars),
            '\n' => lexer.end_line(&mut chars),
            ';' | '|' | '&' => lexer.end_command(),
            c if c.is_whitespace() => lexer.finish_word(),
            _ => lexer.word.push(ch),
        }
    }
    lexer.end_command();
    lexer.commands
}

/// A word the lexer emitted for a redirection operator, with any descriptor
/// prefix: `>`, `>>`, `2>`, `>&`, `<`, `<<`, `<<-`, `<<<`, `<&`. Its
/// following word is the operator's operand, never a program's file operand.
pub(in crate::workflow_read_guard) fn redirect_operator(word: &str) -> bool {
    let bare = word.trim_start_matches(|c: char| c.is_ascii_digit());
    bare.starts_with(['<', '>'])
        && bare
            .chars()
            .all(|c| matches!(c, '<' | '>' | '&' | '-' | '|'))
}

#[derive(Default)]
struct Lexer {
    commands: Vec<Vec<String>>,
    words: Vec<String>,
    word: String,
    /// Set by a `<<`/`<<-` operator: the next plain word is a heredoc
    /// delimiter (`true` when `<<-` strips leading tabs from the body).
    delimiter_pending: Option<bool>,
    /// Heredocs announced on the current line, consumed in order at its end.
    heredocs: Vec<(String, bool)>,
}

impl Lexer {
    fn finish_word(&mut self) {
        if self.word.is_empty() {
            return;
        }
        let word = std::mem::take(&mut self.word);
        if let Some(strip_tabs) = self.delimiter_pending.take() {
            self.heredocs.push((word.clone(), strip_tabs));
        }
        self.words.push(word);
    }

    fn end_command(&mut self) {
        self.finish_word();
        if !self.words.is_empty() {
            self.commands.push(std::mem::take(&mut self.words));
        }
    }

    /// A newline ends the command and, when the line announced heredocs,
    /// their bodies follow in order; each is skipped through its terminator.
    fn end_line(&mut self, chars: &mut Peekable<Chars<'_>>) {
        self.end_command();
        self.delimiter_pending = None;
        for (delimiter, strip_tabs) in std::mem::take(&mut self.heredocs) {
            skip_heredoc_body(chars, &delimiter, strip_tabs);
        }
    }

    /// One redirection operator word: an optional descriptor already in
    /// `word`, `ch` doubled when repeated, `<<-`/`<<<`, and a `&` suffix.
    fn redirect(&mut self, ch: char, chars: &mut Peekable<Chars<'_>>) {
        let fd = if !self.word.is_empty() && self.word.chars().all(|c| c.is_ascii_digit()) {
            std::mem::take(&mut self.word)
        } else {
            self.finish_word();
            String::new()
        };
        let mut op = format!("{fd}{ch}");
        if chars.peek() == Some(&ch) {
            op.push(chars.next().unwrap());
            // `<<<` is a here-string, `<<-` a tab-stripping heredoc.
            if ch == '<' && matches!(chars.peek(), Some('<' | '-')) {
                op.push(chars.next().unwrap());
            }
        }
        if chars.peek() == Some(&'&') {
            op.push(chars.next().unwrap());
        }
        let bare = op.trim_start_matches(|c: char| c.is_ascii_digit());
        if bare == "<<" || bare == "<<-" {
            self.delimiter_pending = Some(bare == "<<-");
        }
        self.words.push(op);
    }
}

/// Consume body lines up to and including the terminator line, or to the end
/// of input when the heredoc is unterminated. Nothing in between is lexed.
fn skip_heredoc_body(chars: &mut Peekable<Chars<'_>>, delimiter: &str, strip_tabs: bool) {
    let mut line = String::new();
    loop {
        let next = chars.next();
        match next {
            Some('\n') | None => {
                let candidate = if strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if candidate == delimiter || next.is_none() {
                    return;
                }
                line.clear();
            }
            Some(c) => line.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_lexed(command: &str, expected: &[&[&str]]) {
        let expected: Vec<Vec<String>> = expected
            .iter()
            .map(|words| words.iter().map(|w| w.to_string()).collect())
            .collect();
        assert_eq!(commands(command), expected, "{command:?}");
    }

    #[test]
    fn a_heredoc_body_is_never_lexed_into_words_or_commands() {
        let command = "cat >> /abs/path/log.md <<'EOF'\nsome prose\nx = > The honest answer is\n\
                       2> /abs/err\ncargo test | tee /abs/x\nrm -rf /\ncd /\ngit stash\nEOF \nEOF\nls";
        assert_lexed(
            command,
            &[&["cat", ">>", "/abs/path/log.md", "<<", "EOF"], &["ls"]],
        );
    }

    #[test]
    fn delimiters_may_be_quoted_unquoted_or_escaped() {
        for command in [
            "cat <<EOF\nrm -rf /\nEOF\nls",
            "cat <<\"EOF\"\nrm -rf /\nEOF\nls",
            "cat <<'EOF'\nrm -rf /\nEOF\nls",
            "cat <<\\EOF\nrm -rf /\nEOF\nls",
            "cat << EOF # note\nrm -rf /\nEOF\nls",
        ] {
            assert_lexed(command, &[&["cat", "<<", "EOF"], &["ls"]]);
        }
    }

    #[test]
    fn a_dash_heredoc_strips_leading_tabs_and_a_plain_one_does_not() {
        assert_lexed(
            "cat <<-EOF\n\trm -rf /\n\t\tEOF\nls",
            &[&["cat", "<<-", "EOF"], &["ls"]],
        );
        assert_lexed(
            "cat <<- 'EOF'\n\trm -rf /\n\tEOF\nls",
            &[&["cat", "<<-", "EOF"], &["ls"]],
        );
        // Without the dash an indented `EOF` is body, not the terminator.
        assert_lexed(
            "cat <<EOF\n\tEOF\nrm -rf /\nEOF\nls",
            &[&["cat", "<<", "EOF"], &["ls"]],
        );
    }

    #[test]
    fn two_heredocs_on_one_line_are_consumed_in_order() {
        assert_lexed(
            "cat <<A; cat <<B\nrm -rf /\nA\nB is not here\n> /abs/x\nB\nls",
            &[&["cat", "<<", "A"], &["cat", "<<", "B"], &["ls"]],
        );
        assert_lexed(
            "diff <<A - <<B\nrm a\nA\nrm b\nB\nls",
            &[&["diff", "<<", "A", "-", "<<", "B"], &["ls"]],
        );
    }

    #[test]
    fn an_unterminated_heredoc_runs_to_the_end_of_input() {
        assert_lexed("cat <<EOF\nrm -rf /\ngit stash\n", &[&["cat", "<<", "EOF"]]);
        assert_lexed("cat <<EOF\nrm -rf /", &[&["cat", "<<", "EOF"]]);
        // No newline after the delimiter: nothing to consume.
        assert_lexed(
            "cat > /abs/x <<EOF",
            &[&["cat", ">", "/abs/x", "<<", "EOF"]],
        );
    }

    #[test]
    fn a_here_string_and_other_redirections_are_not_heredocs() {
        assert_lexed(
            "grep x <<< 'a b'\nls",
            &[&["grep", "x", "<<<", "a b"], &["ls"]],
        );
        assert_lexed(
            "cmd 2>&1 >out <in\nls",
            &[&["cmd", "2>&", "1", ">", "out", "<", "in"], &["ls"]],
        );
    }

    #[test]
    fn redirect_operator_words_are_recognised_and_plain_words_are_not() {
        for word in ["<", ">", ">>", "2>", "2>&", ">&", "<<", "<<-", "<<<", "0<"] {
            assert!(redirect_operator(word), "{word}");
        }
        for word in ["-", "--", "1", "a>b", "s/>/x/", "", "/dev/null", "EOF"] {
            assert!(!redirect_operator(word), "{word}");
        }
    }
}
