pub(super) fn extract_sections(body: &str, headings: &[&str]) -> Vec<String> {
    extract_raw_sections(body, headings)
        .into_iter()
        .flat_map(markdown_list_items)
        .collect()
}

pub(super) fn extract_raw_sections<'a>(body: &'a str, headings: &[&str]) -> Vec<&'a str> {
    let mut sections = Vec::new();
    let lines = body
        .split_inclusive('\n')
        .scan(0usize, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        })
        .collect::<Vec<_>>();

    let mut idx = 0usize;
    while idx < lines.len() {
        let (start, line) = lines[idx];
        let Some((level, title)) = markdown_heading(line.trim()) else {
            idx += 1;
            continue;
        };
        if !headings
            .iter()
            .any(|heading| title.eq_ignore_ascii_case(heading))
        {
            idx += 1;
            continue;
        }

        let content_start = start + line.len();
        let mut end = body.len();
        for (next_start, next_line) in lines.iter().skip(idx + 1) {
            if let Some((next_level, _)) = markdown_heading(next_line.trim())
                && next_level <= level
            {
                end = *next_start;
                break;
            }
        }
        sections.push(body[content_start..end].trim());
        idx += 1;
    }
    sections
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if hashes < 2 || !line.get(hashes..)?.starts_with(' ') {
        return None;
    }
    Some((hashes, line[hashes..].trim()))
}

fn markdown_list_items(section: &str) -> Vec<String> {
    section
        .lines()
        .map(|line| line.trim().trim_start_matches('-').trim())
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}
