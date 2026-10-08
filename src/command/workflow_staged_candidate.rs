use anyhow::Result;

/// Normalize one authored TASK candidate, preserving clean outer-fence support.
pub(super) fn normalize_task_candidate(
    candidate: Vec<u8>,
) -> Result<(Vec<u8>, bool), &'static str> {
    let text = std::str::from_utf8(&candidate).map_err(|_| "the task file is not UTF-8")?;
    if crate::command::topology_lint::outer_fence_with_surrounding_text(text) == Some(true) {
        return Err("the answer has text before/after the task file");
    }
    let (candidate, unwrapped) = unwrap_outer_fence(candidate);
    let text = std::str::from_utf8(&candidate).map_err(|_| "the task file is not UTF-8")?;
    if !text.starts_with("```yaml\n")
        && !text.starts_with("```yaml\r\n")
        && !text.starts_with("```yml\n")
        && !text.starts_with("```yml\r\n")
    {
        return Err("the task file does not start with its own frontmatter");
    }
    Ok((candidate, unwrapped))
}

/// Remove the established pure whole-document fence, if present.
pub(super) fn unwrap_outer_fence(candidate: Vec<u8>) -> (Vec<u8>, bool) {
    match std::str::from_utf8(&candidate)
        .ok()
        .and_then(crate::command::topology_lint::unwrap_outer_fence)
    {
        Some(inner) => (inner.as_bytes().to_vec(), true),
        None => (candidate, false),
    }
}
