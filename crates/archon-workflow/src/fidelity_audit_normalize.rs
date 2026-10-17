//! Markdown-aware quote normalization for provenance checks.

/// Normalize quote punctuation and Markdown wrappers. NFKC is omitted because
/// this crate has no direct or workspace normalization dependency; quote and
/// dash forms, whitespace and formatting remain deliberately bounded.
pub(super) fn normalize_text(text: &str, is_quote: bool) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut segments = Vec::<(bool, String)>::new();
    let mut prose = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '`' {
            if let Some((content_start, close_start, close_end)) =
                fenced_code_span(&chars, index).or_else(|| code_span(&chars, index))
            {
                if !prose.is_empty() {
                    segments.push((false, normalize_prose(&prose)));
                    prose.clear();
                }
                segments.push((true, chars[content_start..close_start].iter().collect()));
                index = close_end;
                continue;
            }
        }
        prose.push(chars[index]);
        index += 1;
    }
    if !prose.is_empty() {
        segments.push((false, normalize_prose(&prose)));
    }
    let has_code = segments.iter().any(|(code, _)| *code);
    let mut collapsed = String::with_capacity(text.len());
    let mut pending_space = false;
    for (code, segment) in segments {
        if code {
            if pending_space && !collapsed.is_empty() {
                collapsed.push(' ');
            }
            pending_space = false;
            collapsed.push_str(&segment);
            continue;
        }
        for ch in segment.chars() {
            if ch.is_whitespace() {
                pending_space = true;
            } else {
                if pending_space && !collapsed.is_empty() {
                    collapsed.push(' ');
                }
                pending_space = false;
                collapsed.push(ch);
            }
        }
    }
    if is_quote && !has_code {
        let opens = collapsed.matches('[').count();
        let closes = collapsed.matches(']').count();
        if closes > opens {
            collapsed.retain(|ch| ch != ']');
        } else if opens > closes {
            collapsed.retain(|ch| ch != '[');
        }
        collapsed = collapsed
            .trim_matches(|ch: char| {
                ch.is_ascii_punctuation() || matches!(ch, '…' | '“' | '”' | '’' | '‘')
            })
            .to_string();
    }
    collapsed
}

fn fenced_code_span(chars: &[char], start: usize) -> Option<(usize, usize, usize)> {
    let line_start = chars[..start]
        .iter()
        .rposition(|ch| *ch == '\n')
        .map_or(0, |newline| newline + 1);
    if start - line_start > 3 || chars[line_start..start].iter().any(|ch| *ch != ' ') {
        return None;
    }
    let mut content_start = start + 1;
    while chars.get(content_start) == Some(&'`') {
        content_start += 1;
    }
    let fence_len = content_start - start;
    if fence_len < 3 {
        return None;
    }
    let opening_end = chars[content_start..]
        .iter()
        .position(|ch| *ch == '\n')
        .map(|offset| content_start + offset + 1)?;
    let mut candidate_line = opening_end;
    while candidate_line < chars.len() {
        let line_end = chars[candidate_line..]
            .iter()
            .position(|ch| *ch == '\n')
            .map_or(chars.len(), |offset| candidate_line + offset);
        let mut marker = candidate_line;
        while marker < line_end && marker - candidate_line < 3 && chars[marker] == ' ' {
            marker += 1;
        }
        let mut close_end = marker;
        while close_end < line_end && chars[close_end] == '`' {
            close_end += 1;
        }
        if close_end - marker >= fence_len
            && chars[close_end..line_end]
                .iter()
                .all(|ch| ch.is_whitespace())
        {
            return Some((content_start, marker, close_end));
        }
        candidate_line = line_end.saturating_add(1);
    }
    Some((content_start, chars.len(), chars.len()))
}

fn code_span(chars: &[char], start: usize) -> Option<(usize, usize, usize)> {
    let mut content_start = start + 1;
    while chars.get(content_start) == Some(&'`') {
        content_start += 1;
    }
    let delimiter_len = content_start - start;
    let mut search = content_start;
    while search < chars.len() {
        if chars[search] != '`' {
            search += 1;
            continue;
        }
        let mut close_end = search + 1;
        while chars.get(close_end) == Some(&'`') {
            close_end += 1;
        }
        if close_end - search == delimiter_len {
            return Some((content_start, search, close_end));
        }
        search = close_end;
    }
    None
}

/// Normalize prose punctuation and Markdown emphasis without touching code.
fn normalize_prose(chars: &[char]) -> String {
    let paired_emphasis = paired_emphasis_markers(chars);
    let mut normalized = String::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\' && chars.get(index + 1) == Some(&'n') {
            normalized.push(' ');
            index += 2;
            continue;
        }
        match chars[index] {
            '‘' | '’' => {
                let previous = index.checked_sub(1).and_then(|i| chars.get(i));
                let next = chars.get(index + 1);
                if previous.is_some_and(|ch| ch.is_alphanumeric())
                    && next.is_some_and(|ch| ch.is_alphanumeric())
                {
                    normalized.push('\'');
                } else {
                    normalized.push('"');
                }
            }
            '“' | '”' | '«' | '»' | '‹' | '›' => normalized.push('"'),
            '–' | '—' | '‑' | '−' => normalized.push('-'),
            '*' | '_' => {
                let marker = chars[index];
                let mut end = index + 1;
                while chars.get(end) == Some(&marker) {
                    end += 1;
                }
                let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
                let next = chars.get(end).copied();
                let can_open = next.is_some_and(|ch| !ch.is_whitespace());
                let can_close = previous.is_some_and(|ch| !ch.is_whitespace());
                let boundary_emphasis = previous
                    .is_none_or(|ch| ch.is_whitespace() || ch.is_ascii_punctuation())
                    || next.is_none_or(|ch| ch.is_whitespace() || ch.is_ascii_punctuation());
                if paired_emphasis[index]
                    && (can_open || can_close)
                    && (marker == '*' || boundary_emphasis)
                {
                    index = end;
                } else {
                    normalized.extend(chars[index..end].iter());
                    index = end;
                }
                continue;
            }
            ch => normalized.push(ch),
        }
        index += 1;
    }
    normalized
}

fn paired_emphasis_markers(chars: &[char]) -> Vec<bool> {
    let mut paired = vec![false; chars.len()];
    let mut openings = Vec::<(char, usize, usize)>::new();
    let mut index = 0;
    while index < chars.len() {
        let marker = chars[index];
        if !matches!(marker, '*' | '_') {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while chars.get(end) == Some(&marker) {
            end += 1;
        }
        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(end).copied();
        let can_open = next.is_some_and(|ch| !ch.is_whitespace());
        let can_close = previous.is_some_and(|ch| !ch.is_whitespace());
        let opening = openings
            .iter()
            .rposition(|(open_marker, run, _)| *open_marker == marker && *run == end - index);
        if can_close {
            if let Some(open_index) = opening {
                let (_, _, open_start) = openings.remove(open_index);
                paired[open_start..open_start + (end - index)].fill(true);
                paired[index..end].fill(true);
            } else if can_open {
                openings.push((marker, end - index, index));
            }
        } else if can_open {
            openings.push((marker, end - index, index));
        }
        index = end;
    }
    paired
}
