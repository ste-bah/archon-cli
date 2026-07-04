fn extract_array_prop(args: &str, key: &str) -> Option<String> {
    extract_balanced_prop(args, key, '[', ']')
}

fn extract_object_prop(args: &str, key: &str) -> Option<String> {
    extract_balanced_prop(args, key, '{', '}')
}

fn extract_balanced_prop(args: &str, key: &str, open: char, close: char) -> Option<String> {
    let regex = Regex::new(&format!(
        r#"(?s)(?:{}|["']{}["'])\s*:\s*{}"#,
        regex::escape(key),
        regex::escape(key),
        regex::escape(&open.to_string())
    ))
    .ok()?;
    let hit = regex.find(args)?;
    let start = hit.end() - open.len_utf8();
    balanced_slice(args, start, open, close)
}

fn balanced_slice(args: &str, start: usize, open: char, close: char) -> Option<String> {
    let mut depth = 0usize;
    let mut quote = None::<char>;
    let mut escaped = false;
    let mut body_start = None;
    for (offset, ch) in args[start..].char_indices() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => quote = Some(ch),
            ch if ch == open => {
                depth += 1;
                if depth == 1 {
                    body_start = Some(start + offset + ch.len_utf8());
                }
            }
            ch if ch == close => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return body_start.map(|inner| args[inner..start + offset].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn split_object_literals(array_body: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let mut depth = 0usize;
    let mut quote = None::<char>;
    let mut escaped = false;
    let mut start = None::<usize>;
    for (idx, ch) in array_body.char_indices() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => quote = Some(ch),
            '{' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0
                    && let Some(start) = start.take()
                {
                    objects.push(array_body[start..=idx].to_string());
                }
            }
            _ => {}
        }
    }
    objects
}
