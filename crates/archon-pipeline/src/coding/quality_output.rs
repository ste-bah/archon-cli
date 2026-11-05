//! Syntax-independent output completeness. This is not a compiler, coverage,
//! wiring, or documentation verdict; those require project tool evidence.

pub fn completeness(output: &str) -> f64 {
    if output.trim().is_empty() {
        return 0.0;
    }
    let mut fence: Option<(char, usize)> = None;
    let mut code = String::new();
    for line in output.lines() {
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().filter(|ch| matches!(ch, '`' | '~'));
        if let Some(marker) = marker {
            let length = trimmed.chars().take_while(|ch| *ch == marker).count();
            if length >= 3 {
                if let Some((open, size)) = fence {
                    if open == marker && length >= size && trimmed[length..].trim().is_empty() {
                        fence = None;
                        continue;
                    }
                } else {
                    fence = Some((marker, length));
                    continue;
                }
            }
        }
        if fence.is_some() {
            code.push_str(line);
            code.push('\n');
        }
    }
    // Narrative may describe unfinished prior work, so inspect code when
    // present and otherwise only whether the requested output was supplied.
    let unfinished = code
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .filter(|word| {
            ["TODO", "FIXME", "unimplemented", "placeholder"]
                .iter()
                .any(|marker| word.eq_ignore_ascii_case(marker))
        })
        .count();
    let score = (1.0 - unfinished as f64 * 0.15).max(0.0);
    if fence.is_some() {
        score.min(0.5)
    } else {
        score
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completeness_is_independent_of_syntax_and_fence_label() {
        for (label, body) in [
            ("rust", "fn answer() -> i32 { 42 }"),
            ("python", "def answer(): return 42"),
            ("go", "func answer() int { return 42 }"),
            ("java", "class Answer {}"),
            ("custom", "answer := 42"),
            ("", "answer := 42"),
        ] {
            assert_eq!(completeness(&format!("```{label}\n{body}\n```")), 1.0);
            assert!(completeness(&format!("```{label}\nTODO\n```")) < 1.0);
        }
        assert_eq!(completeness(""), 0.0);
        assert_eq!(completeness("Design rationale and evidence."), 1.0);
        assert!(completeness("```custom\nunclosed") < 1.0);
    }
}
