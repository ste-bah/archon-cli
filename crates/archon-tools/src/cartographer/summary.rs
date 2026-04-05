use super::index::CodebaseIndex;

/// Generate a token-budget-aware textual summary of the codebase index.
///
/// `max_tokens` is converted to a character budget (`max_tokens * 4`).
/// Files are listed in sorted order.  Truncates with `"  ...\n"` when the
/// budget would be exceeded.
pub fn generate_summary(index: &CodebaseIndex, max_tokens: usize) -> String {
    let max_chars = max_tokens * 4;
    let mut out = String::new();

    let mut files: Vec<&str> = index.symbols.keys().map(|s| s.as_str()).collect();
    files.sort_unstable();

    for file in files {
        let syms = match index.symbols.get(file) {
            Some(s) => s,
            None => continue,
        };

        if syms.is_empty() {
            continue;
        }

        let header = format!("## {file}\n");
        if out.len() + header.len() > max_chars {
            out.push_str("  ...\n");
            return out;
        }
        out.push_str(&header);

        for sym in syms {
            let line = format!("  [{:?}] {} (line {})\n", sym.kind, sym.signature, sym.line);
            if out.len() + line.len() > max_chars {
                out.push_str("  ...\n");
                return out;
            }
            out.push_str(&line);
        }

        out.push('\n');

        if out.len() > max_chars {
            return out;
        }
    }

    out
}
