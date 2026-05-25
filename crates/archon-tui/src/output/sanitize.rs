//! Transcript text sanitizer.

pub(super) fn sanitize_output_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            strip_escape_sequence(&mut chars);
            continue;
        }
        match ch {
            '\r' | '\n' => out.push('\n'),
            '\t' => out.push_str("    "),
            ch if ch.is_control() => {}
            ch => out.push(ch),
        }
    }
    out
}

fn strip_escape_sequence<I>(chars: &mut std::iter::Peekable<I>)
where
    I: Iterator<Item = char>,
{
    match chars.peek().copied() {
        Some('[') => {
            chars.next();
            for ch in chars.by_ref() {
                if ('@'..='~').contains(&ch) {
                    break;
                }
            }
        }
        Some(']') => {
            chars.next();
            while let Some(ch) = chars.next() {
                if ch == '\x07' {
                    break;
                }
                if ch == '\x1b' && matches!(chars.peek(), Some('\\')) {
                    chars.next();
                    break;
                }
            }
        }
        Some(_) => {
            chars.next();
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_output_text;

    #[test]
    fn sanitizer_strips_terminal_control_sequences() {
        let raw = "ok\x1b[31m red\x1b[0m\rnext\tcol\x08";
        assert_eq!(sanitize_output_text(raw), "ok red\nnext    col");
    }
}
