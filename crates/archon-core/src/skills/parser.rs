/// Parse a slash command string into (command_name, arguments).
///
/// Returns `None` if the input does not start with `/` or is empty.
/// Handles double-quoted strings so that e.g.
/// `/export "my file.md" --format json` yields `("export", ["my file.md", "--format", "json"])`.
pub fn parse_slash_command(input: &str) -> Option<(String, Vec<String>)> {
    let trimmed = input.trim();
    if trimmed.is_empty() || !trimmed.starts_with('/') {
        return None;
    }

    let without_slash = &trimmed[1..];
    let tokens = tokenize(without_slash);
    if tokens.is_empty() {
        return None;
    }

    let command = tokens[0].clone();
    let args = tokens[1..].to_vec();
    Some((command, args))
}

/// Tokenize a string respecting double-quoted segments.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let chars = input.chars().peekable();

    for ch in chars {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
            }
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_simple() {
        assert_eq!(tokenize("a b c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn tokenize_quoted() {
        assert_eq!(
            tokenize(r#"export "my file.md" --format json"#),
            vec!["export", "my file.md", "--format", "json"]
        );
    }
}
