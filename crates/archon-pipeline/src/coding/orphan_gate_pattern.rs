use regex::Regex;

pub(super) fn reference_pattern(stem: &str) -> Result<Regex, regex::Error> {
    let stem = regex::escape(stem);
    let module_end = r"(?:$|[^A-Za-z0-9_-])";
    let go_import_name = r"(?:[A-Za-z_][A-Za-z0-9_]*|[_.])";
    let path = format!(r#"["'](?:[^/"']*/)*{stem}(?:\.[A-Za-z0-9_]+)?["']"#);
    Regex::new(&format!(
        r#"(?xs)
            \bmod\s+{stem}{module_end}
            | \buse\s+[^;]*?(?:^|[:{{,\s]){stem}{module_end}
            | \bfrom\s+(?:\.+)?(?:[A-Za-z_][A-Za-z0-9_]*\.)*{stem}\s+import\b
            | (?:\bimport|,)\s*(?:[A-Za-z_][A-Za-z0-9_]*\.)*{stem}(?:$|[\s,])
            | \bimport\s*(?:\(\s*)?(?:[^;]*?\bfrom\s+)?{path}
            | \bimport\s+{go_import_name}\s+{path}
            | \bimport\s*\(\s*(?:(?:{go_import_name}\s+)?["'][^"'\r\n]+["']\s*)*(?:{go_import_name}\s+)?{path}
            | \brequire\s*\(\s*{path}
        "#
    ))
}

pub(super) fn generic_reference_pattern(stem: &str) -> Result<Regex, regex::Error> {
    Regex::new(&format!(
        r"(?:^|[^A-Za-z0-9_.-])(?:[A-Za-z_][A-Za-z0-9_]*\.)*{}(?:$|[^A-Za-z0-9_.-])",
        regex::escape(stem)
    ))
}
