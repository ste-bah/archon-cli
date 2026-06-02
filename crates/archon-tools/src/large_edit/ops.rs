#[derive(Debug, Clone)]
struct LineSpan<'a> {
    index: usize,
    start: usize,
    end: usize,
    text: &'a str,
}

pub(super) fn insert_after(
    original: &str,
    anchor: &str,
    content: &str,
    occurrence: usize,
) -> Result<(String, String), String> {
    let lines = line_spans(original);
    let anchor_line = find_line(&lines, anchor, occurrence)?;
    let mut insert = normalize_block(content);
    if !anchor_line.text.ends_with('\n') && !insert.starts_with('\n') && !insert.is_empty() {
        insert.insert(0, '\n');
    }

    let mut updated = String::with_capacity(original.len() + insert.len());
    updated.push_str(&original[..anchor_line.end]);
    updated.push_str(&insert);
    updated.push_str(&original[anchor_line.end..]);
    Ok((
        updated,
        format!(
            "Inserted {} bytes after anchor '{}' at line {}.",
            content.len(),
            anchor,
            anchor_line.index + 1
        ),
    ))
}

pub(super) fn replace_section(
    original: &str,
    start_anchor: &str,
    end_anchor: Option<&str>,
    content: &str,
    occurrence: usize,
) -> Result<(String, String), String> {
    let lines = line_spans(original);
    let start = find_line(&lines, start_anchor, occurrence)?;
    let end = section_end(original, &lines, start.index, end_anchor)?;
    let replacement = normalize_block(content);

    let mut updated = String::with_capacity(original.len() + replacement.len());
    updated.push_str(&original[..start.start]);
    updated.push_str(&replacement);
    updated.push_str(&original[end..]);
    Ok((
        updated,
        format!(
            "Replaced section starting at anchor '{}' on line {} ({} bytes inserted).",
            start_anchor,
            start.index + 1,
            content.len()
        ),
    ))
}

pub(super) fn delete_section(
    original: &str,
    start_anchor: &str,
    end_anchor: Option<&str>,
    occurrence: usize,
) -> Result<(String, String), String> {
    let lines = line_spans(original);
    let start = find_line(&lines, start_anchor, occurrence)?;
    let end = section_end(original, &lines, start.index, end_anchor)?;

    let mut updated = String::with_capacity(original.len());
    updated.push_str(&original[..start.start]);
    updated.push_str(&original[end..]);
    Ok((
        updated,
        format!(
            "Deleted section starting at anchor '{}' on line {}.",
            start_anchor,
            start.index + 1
        ),
    ))
}

fn line_spans(content: &str) -> Vec<LineSpan<'_>> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, line) in content.split_inclusive('\n').enumerate() {
        let end = start + line.len();
        spans.push(LineSpan {
            index,
            start,
            end,
            text: line,
        });
        start = end;
    }
    spans
}

fn find_line<'a>(
    lines: &'a [LineSpan<'a>],
    anchor: &str,
    occurrence: usize,
) -> Result<&'a LineSpan<'a>, String> {
    if anchor.trim().is_empty() {
        return Err("anchor must not be empty".into());
    }
    let wanted = occurrence.max(1);
    let mut seen = 0;
    for line in lines {
        if line.text.contains(anchor) {
            seen += 1;
            if seen == wanted {
                return Ok(line);
            }
        }
    }
    Err(format!(
        "anchor '{}' occurrence {} was not found",
        anchor, wanted
    ))
}

fn section_end(
    original: &str,
    lines: &[LineSpan<'_>],
    start_index: usize,
    end_anchor: Option<&str>,
) -> Result<usize, String> {
    if let Some(anchor) = end_anchor.filter(|a| !a.trim().is_empty()) {
        for line in lines.iter().skip(start_index + 1) {
            if line.text.contains(anchor) {
                return Ok(line.start);
            }
        }
        return Err(format!(
            "end_anchor '{anchor}' was not found after start_anchor"
        ));
    }

    let Some(start_line) = lines.get(start_index) else {
        return Ok(original.len());
    };
    let Some(start_heading) = markdown_heading_level(start_line.text) else {
        return Ok(start_line.end);
    };
    for line in lines.iter().skip(start_index + 1) {
        if let Some(level) = markdown_heading_level(line.text)
            && level <= start_heading
        {
            return Ok(line.start);
        }
    }
    Ok(original.len())
}

fn markdown_heading_level(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = trimmed.get(hashes..)?;
    if rest.starts_with(' ') {
        Some(hashes)
    } else {
        None
    }
}

fn normalize_block(content: &str) -> String {
    if content.is_empty() || content.ends_with('\n') {
        content.to_string()
    } else {
        format!("{content}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_markdown_section_until_next_peer_heading() {
        let original = "# A\nold\n## B\nkeep\n# C\nend\n";
        let (updated, _) = replace_section(original, "# A", None, "# A\nnew\n", 1).unwrap();
        assert_eq!(updated, "# A\nnew\n# C\nend\n");
    }

    #[test]
    fn inserts_after_anchor_line() {
        let original = "one\ntwo\n";
        let (updated, _) = insert_after(original, "one", "added", 1).unwrap();
        assert_eq!(updated, "one\nadded\ntwo\n");
    }

    #[test]
    fn deletes_between_explicit_anchors() {
        let original = "a\nSTART\nx\nEND\nb\n";
        let (updated, _) = delete_section(original, "START", Some("END"), 1).unwrap();
        assert_eq!(updated, "a\nEND\nb\n");
    }
}
