pub(super) fn headings_match(found: &str, requested: &str) -> bool {
    if found.is_empty() {
        return false;
    }
    let found = found.to_ascii_lowercase();
    let requested = requested.to_ascii_lowercase();
    if found == requested {
        return true;
    }
    let (shorter, longer) = if found.len() < requested.len() {
        (&found, &requested)
    } else {
        (&requested, &found)
    };
    // `as_bytes().get(..) == Some(&b' ')` rather than `chars().nth`: the prefix
    // is ASCII-compared already, and a space is one byte in UTF-8, so this
    // cannot split a multi-byte character.
    longer.starts_with(shorter.as_str()) && longer.as_bytes().get(shorter.len()) == Some(&b' ')
}

pub(super) fn section_bullets(raw: &str, heading: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut inside = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let found = rest.trim_start_matches('#').trim();
            // Either heading may be a prefix of the other, because authors go
            // wrong in both directions and both directions silently yield zero
            // bullets.
            //
            // Shorter than requested: `## Files Expected` where the contract
            // says `## Files Expected to Change`. The task then parses as
            // owning NOTHING and every path it declared is invisible. Observed
            // live: one generated spec in three used the short form, listing
            // four real files the report then called "declares no paths".
            //
            // Longer than requested: `## Focused Tests and Evidence` where the
            // contract says `## Focused Tests`. Observed live in the same
            // corpus — "exact-heading parsing found three fatal section-name
            // defects (`Focused Tests and …` is ignored)" — and the writers
            // spent a remediation round renaming headings to satisfy this
            // parser. A reader that forces the document to change is the wrong
            // way round.
            //
            // The boundary check is what keeps this from being a substring
            // match: the shorter must end where the longer has a space, so
            // `Files Forbidden…` can never satisfy `Files Expected…` and
            // `Focused Testing` can never satisfy `Focused Tests`.
            inside = headings_match(found, heading);
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let item = item.trim();
            if !item.is_empty() {
                items.push(item.to_string());
            }
        }
    }
    items
}
