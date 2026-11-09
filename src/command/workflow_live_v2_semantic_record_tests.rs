pub(super) fn persisted_semantic_rejection_record(root: &std::path::Path, repair_id: &str) -> bool {
    fn walk(dir: &std::path::Path, needle: &str) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if walk(&path, needle) {
                    return true;
                }
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(needle))
            {
                return true;
            }
        }
        false
    }
    walk(root, &format!("{repair_id}-semantic-preservation-rejected"))
}
