pub fn append_host_note(mut stdout: String, fields: &[String]) -> (String, u64) {
    if !fields.is_empty() {
        stdout.push_str("\nHost note: replaced differing frozen frontmatter fields (authored values redacted): ");
        stdout.push_str(&fields.join(", "));
        stdout.push('.');
    }
    let byte_count = stdout.len() as u64;
    (stdout, byte_count)
}
