use std::path::Path;

fn scan(path: &Path, failures: &mut Vec<String>) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().unwrap() != "tests" {
                scan(&path, failures);
            }
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap();
        if !name.ends_with(".rs")
            || (name.ends_with("_tests.rs") || name == "tests.rs")
            || name == "provider_fault.rs"
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let mut test_item = false;
        let mut depth = 0isize;
        let mut opened = false;
        for (index, line) in text.lines().enumerate() {
            if line.trim() == "#[cfg(test)]" {
                test_item = true;
                opened = false;
                depth = 0;
                continue;
            }
            if test_item {
                opened |= line.contains('{');
                depth += line.matches('{').count() as isize - line.matches('}').count() as isize;
                if (opened && depth <= 0) || (!opened && line.trim_end().ends_with(';')) {
                    test_item = false;
                }
                continue;
            }
            if !line.trim_start().starts_with("//")
                && line
                    .rsplit_once("=>")
                    .map_or(line, |(_, expression)| expression)
                    .contains("LlmError::Http(")
            {
                failures.push(format!("{}:{}: {}", path.display(), index + 1, line.trim()));
            }
        }
    }
}

#[test]
fn untyped_http_construction_is_confined_to_shared_conversion() {
    let mut failures = Vec::new();
    scan(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut failures,
    );
    assert!(
        failures.is_empty(),
        "untyped provider faults bypass shared conversion:\n{}",
        failures.join("\n")
    );
}
