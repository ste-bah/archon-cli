pub fn append_host_note(mut stdout: String, fields: &[String], child_bytes: u64) -> (String, u64) {
    let original_len = stdout.len();
    if !fields.is_empty() {
        stdout.push_str("\nHost note: replaced differing frozen frontmatter fields (authored values redacted): ");
        stdout.push_str(&fields.join(", "));
        stdout.push('.');
    }
    let byte_count = child_bytes + (stdout.len() - original_len) as u64;
    (stdout, byte_count)
}
