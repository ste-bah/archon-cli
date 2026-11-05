//! Shared block preparation for document-derived inventories and section keys.
//! ATX and Setext headings have the same representation; fenced examples never
//! become declarations. Line counts stay stable for source-section lookup.
pub(super) fn prepare(source: &str, hide_code: bool) -> String {
    let mut lines: Vec<String> = source.lines().map(str::to_string).collect();
    let mut fence = None;
    let mut previous: Option<usize> = None;
    for index in 0..lines.len() {
        let trimmed = lines[index].trim_start();
        let marker = trimmed.chars().next().unwrap_or(' ');
        let width = trimmed.chars().take_while(|ch| *ch == marker).count();
        if matches!(marker, '`' | '~') && width >= 3 {
            match fence {
                None => fence = Some((marker, width)),
                Some((open, count))
                    if open == marker && width >= count && trimmed[width..].trim().is_empty() =>
                {
                    fence = None
                }
                _ => {}
            }
            if hide_code {
                lines[index].clear();
            }
            previous = None;
            continue;
        }
        if fence.is_some() {
            if hide_code {
                lines[index].clear();
            }
            continue;
        }
        if let Some(prior) = previous {
            let underline = trimmed.trim();
            if !underline.is_empty()
                && (underline.chars().all(|ch| ch == '=') || underline.chars().all(|ch| ch == '-'))
            {
                let level = if underline.starts_with('=') {
                    "#"
                } else {
                    "##"
                };
                lines[prior] = format!("{level} {}", lines[prior].trim());
                lines[index].clear();
                previous = None;
                continue;
            }
        }
        previous = (!trimmed.is_empty()
            && !trimmed.starts_with(['#', '|', '>', '<'])
            && !super::is_list_item(trimmed)
            && !super::is_setext_underline(trimmed))
        .then_some(index);
    }
    lines.join("\n")
}
