//! Composing the text handed to the agent for one `archon/prompt`
//! (issue #26, `contextFiles`).
//!
//! The first slice named attached files without reading them, because the
//! agent had no tools and could not open them either. Now that it does, they
//! are read here instead: an editor attachment is a statement about what the
//! user is looking at, and making the agent spend a `Read` round-trip to find
//! that out is slower and needs a permission decision for something the user
//! already chose to share.
//!
//! A file that cannot be read is reported in the prompt rather than dropped.
//! Silently omitting it produces the worst outcome — the model answers
//! confidently about a file it never saw.

use std::path::Path;

use crate::ide::protocol::IdePromptParams;

/// Largest slice of any one attachment inlined into the prompt.
///
/// Bounded because the caller is an editor and nothing stops it attaching a
/// 40 MB log. The path is still named, so the agent can `Read` the rest with
/// an offset if it needs to.
pub const CONTEXT_FILE_BYTE_LIMIT: usize = 64 * 1024;

/// Build the text handed to the agent for one prompt.
pub fn compose_prompt(params: &IdePromptParams) -> String {
    let files = params.context_files.as_deref().unwrap_or(&[]);
    if files.is_empty() {
        return params.text.clone();
    }

    let mut out = params.text.clone();
    out.push_str("\n\nFiles attached in the editor:");
    for file in files {
        out.push_str("\n\n--- ");
        out.push_str(file);
        out.push_str(" ---\n");
        out.push_str(&read_attachment(Path::new(file)));
    }
    out
}

/// Read one attachment, or describe why it could not be read.
fn read_attachment(path: &Path) -> String {
    match std::fs::read(path) {
        Err(error) => format!("[not readable: {error}]"),
        Ok(bytes) => {
            let truncated = bytes.len() > CONTEXT_FILE_BYTE_LIMIT;
            let head = if truncated {
                &bytes[..CONTEXT_FILE_BYTE_LIMIT]
            } else {
                &bytes[..]
            };
            // Lossy rather than a hard failure: a file with one stray byte is
            // still worth showing, and an editor can attach anything.
            let mut text = String::from_utf8_lossy(head).into_owned();
            if truncated {
                text.push_str(&format!(
                    "\n[truncated at {CONTEXT_FILE_BYTE_LIMIT} bytes of {}; read the file for the rest]",
                    bytes.len()
                ));
            }
            text
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(text: &str, files: Option<Vec<String>>) -> IdePromptParams {
        IdePromptParams {
            session_id: "s".to_string(),
            text: text.to_string(),
            context_files: files,
        }
    }

    #[test]
    fn prompt_without_attachments_is_passed_through_verbatim() {
        assert_eq!(
            compose_prompt(&params("explain this", None)),
            "explain this"
        );
    }

    #[test]
    fn an_attachment_is_read_into_the_prompt() {
        let dir = std::env::temp_dir().join(format!("archon-ide-ctx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("attached.rs");
        std::fs::write(&file, "fn answer() -> u8 { 42 }").expect("write");

        let composed = compose_prompt(&params(
            "explain this",
            Some(vec![file.display().to_string()]),
        ));

        assert!(composed.starts_with("explain this"));
        assert!(composed.contains("attached.rs"));
        assert!(
            composed.contains("fn answer() -> u8 { 42 }"),
            "attachment contents must reach the agent: {composed}"
        );
        std::fs::remove_file(&file).ok();
    }

    /// The failure mode this guards is a confident answer about a file the
    /// model never saw, which is what dropping the attachment silently buys.
    #[test]
    fn an_unreadable_attachment_is_reported_rather_than_dropped() {
        let missing = std::env::temp_dir().join("archon-ide-ctx-does-not-exist.rs");
        std::fs::remove_file(&missing).ok();

        let composed = compose_prompt(&params(
            "explain this",
            Some(vec![missing.display().to_string()]),
        ));

        assert!(composed.contains("archon-ide-ctx-does-not-exist.rs"));
        assert!(composed.contains("not readable"), "{composed}");
    }

    #[test]
    fn an_oversized_attachment_is_truncated_and_says_so() {
        let dir = std::env::temp_dir().join(format!("archon-ide-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("big.txt");
        std::fs::write(&file, "x".repeat(CONTEXT_FILE_BYTE_LIMIT + 10)).expect("write");

        let composed = compose_prompt(&params("summarise", Some(vec![file.display().to_string()])));

        assert!(composed.contains("truncated at"), "{composed}");
        std::fs::remove_file(&file).ok();
    }
}
