//! Preprocessor conditionals for the hand lexer.

/// Preprocessor conditionals: only the first branch of each `#if` /
/// `#ifdef` / `#ifndef` is read (none of `#if 0`, whose `#else` is read
/// instead). Branches written as alternatives usually open the same braces,
/// so reading every one of them unbalanced the scan.
#[derive(Debug, Default)]
pub(super) struct Conditionals {
    /// Per open conditional: (this branch is skipped, a branch was taken).
    frames: Vec<(bool, bool)>,
}

impl Conditionals {
    pub(super) fn skipping(&self) -> bool {
        self.frames.iter().any(|(skipped, _)| *skipped)
    }

    pub(super) fn directive(&mut self, line: &str) {
        let rest = line.trim_start().trim_start_matches('#').trim_start();
        let word: String = rest.chars().take_while(char::is_ascii_alphabetic).collect();
        // `#if 0 /* off */`, `#if (0)`: the condition without comments or
        // enclosing parentheses.
        let mut argument = rest[word.len()..]
            .split("/*")
            .next()
            .and_then(|code| code.split("//").next())
            .unwrap_or("")
            .trim();
        while let Some(inner) = argument
            .strip_prefix('(')
            .and_then(|inner| inner.strip_suffix(')'))
        {
            argument = inner.trim();
        }
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
