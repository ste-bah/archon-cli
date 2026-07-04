fn parse_depends_on(args: &str) -> Option<Vec<String>> {
    static DEPS_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"depends_on["']?\s*:\s*\[(?P<deps>[^\]]*)\]"#)
            .expect("depends_on regex compiles")
    });
    let captures = DEPS_RE.captures(args)?;
    let deps = captures
        .name("deps")?
        .as_str()
        .split(',')
        .map(|value| value.trim().trim_matches('"').trim_matches('\''))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!deps.is_empty()).then_some(deps)
}

fn parse_source_var_arg(args: &str) -> Option<String> {
    static SOURCE_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"^\s*,\s*([A-Za-z_][A-Za-z0-9_]*)\s*,"#)
            .expect("source argument regex compiles")
    });
    SOURCE_RE
        .captures(args)
        .and_then(|captures| captures.get(1))
        .map(|m| m.as_str().to_string())
}

fn parse_string_prop(args: &str, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let regex = Regex::new(&format!(
            r#"(?s)(?:{}|["']{}["'])\s*:\s*["']([^"']+)["']"#,
            regex::escape(key),
            regex::escape(key)
        ))
        .ok()?;
        regex
            .captures(args)
            .and_then(|captures| captures.get(1))
            .map(|m| m.as_str().trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn parse_identifier_prop(args: &str, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let regex = Regex::new(&format!(
            r#"(?s)(?:{}|["']{}["'])\s*:\s*([A-Za-z_][A-Za-z0-9_]*)"#,
            regex::escape(key),
            regex::escape(key)
        ))
        .ok()?;
        regex
            .captures(args)
            .and_then(|captures| captures.get(1))
            .map(|m| m.as_str().trim().to_string())
            .filter(|value| !matches!(value.as_str(), "true" | "false" | "null" | "undefined"))
    })
}

fn parse_u32_prop(args: &str, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|key| {
        let regex = Regex::new(&format!(
            r#"(?s)(?:{}|["']{}["'])\s*:\s*(\d+)"#,
            regex::escape(key),
            regex::escape(key)
        ))
        .ok()?;
        regex
            .captures(args)
            .and_then(|captures| captures.get(1))
            .and_then(|m| m.as_str().parse::<u32>().ok())
    })
}

fn parse_bool_prop(args: &str, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| {
        let regex = Regex::new(&format!(
            r#"(?s)(?:{}|["']{}["'])\s*:\s*(true|false)"#,
            regex::escape(key),
            regex::escape(key)
        ))
        .ok()?;
        regex
            .captures(args)
            .and_then(|captures| captures.get(1))
            .and_then(|m| m.as_str().parse::<bool>().ok())
    })
}

fn parse_string_array_prop(args: &str, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| {
            let regex = Regex::new(&format!(
                r#"(?s)(?:{}|["']{}["'])\s*:\s*\[(?P<items>[^\]]*)\]"#,
                regex::escape(key),
                regex::escape(key)
            ))
            .ok()?;
            let captures = regex.captures(args)?;
            Some(
                captures
                    .name("items")?
                    .as_str()
                    .split(',')
                    .map(|value| value.trim().trim_matches('"').trim_matches('\''))
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_default()
}

fn parse_inline_items(args: &str) -> Vec<serde_json::Value> {
    let Some(body) = extract_array_prop(args, "items") else {
        return Vec::new();
    };
    split_object_literals(&body)
        .into_iter()
        .filter_map(|object| {
            let mut item = serde_json::Map::new();
            if let Some(name) = parse_string_prop(&object, &["name", "id", "task_id"]) {
                item.insert("id".into(), serde_json::Value::String(name.clone()));
                item.insert("task_id".into(), serde_json::Value::String(name));
            }
            if let Some(task) = parse_string_prop(&object, &["task"]) {
                item.insert("task".into(), serde_json::Value::String(task));
            }
            let target_files = parse_string_array_prop(
                &object,
                &[
                    "targetFiles",
                    "target_files",
                    "expectedTargetFiles",
                    "expected_target_files",
                ],
            );
            if !target_files.is_empty() {
                item.insert(
                    "target_files".into(),
                    serde_json::Value::Array(
                        target_files
                            .into_iter()
                            .map(serde_json::Value::String)
                            .collect(),
                    ),
                );
            }
            (!item.is_empty()).then_some(serde_json::Value::Object(item))
        })
        .collect()
}

fn parse_item_filter(args: &str) -> Option<String> {
    let body = extract_object_prop(args, "itemFilter")?;
    let (field, values) = [
        ("phase", vec!["phases", "phase"]),
        (
            "task_id",
            vec!["tasks", "task", "task_ids", "taskIds", "task_id", "taskId"],
        ),
        ("id", vec!["ids", "id"]),
    ]
    .into_iter()
    .find_map(|(field, keys)| {
        let values = parse_string_array_prop(&body, &keys);
        (!values.is_empty()).then_some((field, values))
    })?;
    let quoted = values
        .into_iter()
        .map(|value| format!("'{}'", value.replace('\'', "")))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("item.{field} in [{quoted}]"))
}
