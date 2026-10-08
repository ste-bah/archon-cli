//! A body author's chat before the task file is packaging the host removes,
//! exactly: the landed file is the task file's own bytes, and what was
//! discarded is recorded, redacted, in the envelope's report.
use super::super::candidate::Packaging;
use super::{FRONTMATTER, landed, landed_with_packaging, refusal, valid_body};

/// A chat line that carries a secret-shaped word, which the record must
/// never repeat.
const CHAT: &str =
    "I read every listed path; token=s3cr3tvalue was not needed. Here is the task file:";

fn discarded(answer: &str) -> (Vec<u8>, bool, Packaging) {
    let landed = landed_with_packaging(answer.as_bytes());
    let packaging = landed.packaging.expect("packaging was discarded");
    (landed.bytes, landed.unwrapped, packaging)
}

/// Shape A: chat, a blank line, a `markdown` wrapper, the task file, and the
/// wrapper's bare closer as the last line.
#[test]
fn chat_then_a_markdown_wrapper_lands_the_inner_task_file_bytes() {
    let body = valid_body();
    let answer = format!("{CHAT}\n\n```markdown\n{body}```\n");
    let (bytes, unwrapped, packaging) = discarded(&answer);
    assert_eq!(bytes, body.as_bytes(), "the inner file, byte for byte");
    assert!(unwrapped);
    assert_eq!(packaging.lines, 3);
    assert_eq!(packaging.bytes, CHAT.len() + 2 + "```markdown\n".len());
    assert!(packaging.wrapper);
}

/// Shape B: chat, a blank line, then the task file to the end; its last
/// line is body prose and stays.
#[test]
fn chat_then_a_bare_task_file_lands_it_with_its_trailing_prose() {
    let body = format!(
        "{}\nA closing remark that is part of the body.\n",
        valid_body()
    );
    let answer = format!("{CHAT}\n\n{body}");
    let (bytes, unwrapped, packaging) = discarded(&answer);
    assert_eq!(bytes, body.as_bytes());
    assert!(!unwrapped);
    assert_eq!((packaging.lines, packaging.bytes), (2, CHAT.len() + 2));
    assert!(!packaging.wrapper);
}

#[test]
fn the_diagnostic_names_the_counts_and_a_redacted_preview() {
    let answer = format!("{CHAT}\n\n```markdown\n{}```\n", valid_body());
    let (_, _, packaging) = discarded(&answer);
    assert!(packaging.preview.contains("<redacted>"), "{packaging:?}");
    assert!(!packaging.preview.contains("s3cr3tvalue"), "{packaging:?}");
    assert!(packaging.preview.starts_with("I read every listed path;"));
    let report = packaging.report();
    assert!(report.contains("3 leading line(s)"), "{report}");
    assert!(
        report.contains(&format!("{} byte(s)", packaging.bytes)),
        "{report}"
    );
    assert!(
        report.contains("wrapper fence pair removed: yes"),
        "{report}"
    );
    assert!(!report.contains("s3cr3tvalue"), "{report}");

    let long = "word ".repeat(100);
    let (_, _, packaging) = discarded(&format!("{long}\n{}", valid_body()));
    assert_eq!(packaging.preview.chars().count(), 200);
}

#[test]
fn an_answer_without_packaging_lands_unchanged() {
    let body = valid_body();
    assert_eq!(landed(body.as_bytes()), (body.into_bytes(), false));
}

#[test]
fn two_task_files_after_chat_are_refused_with_the_existing_finding() {
    let second = FRONTMATTER.replace("TASK-X-001", "TASK-X-002");
    let answer = format!("{CHAT}\n\n{}\n{second}\n# TASK-X-002\n", valid_body());
    assert_eq!(
        refusal(&answer),
        format!(
            "the answer has text before the task file (first line: \"{CHAT}\"); return only the task file, starting with its ```yaml frontmatter block"
        )
    );
}

#[test]
fn chat_with_no_task_file_is_refused_with_the_existing_finding() {
    let answer = format!("{CHAT}\n\n```yaml\nkey: value\n```\n\nNo task file follows.\n");
    assert!(
        refusal(&answer).starts_with(&format!(
            "the answer has text before the task file (first line: \"{CHAT}\")"
        )),
        "{}",
        refusal(&answer)
    );
}

/// Only a wrapper's own closer is removed: a bare fence after a task file
/// that was not wrapped is body text and lands.
#[test]
fn text_after_an_unwrapped_task_file_is_never_cut() {
    let body = format!("{}\nTrailing prose.\n\n```\n", valid_body());
    let (bytes, unwrapped, _) = discarded(&format!("{CHAT}\n{body}"));
    assert_eq!(bytes, body.as_bytes());
    assert!(!unwrapped);
}

/// A heading before the frontmatter, and chat that ends with its own closed
/// code block, are leading text; the block's closer is not a wrapper.
#[test]
fn a_heading_or_a_closed_chat_block_before_the_task_file_is_discarded() {
    let body = valid_body();
    let (bytes, _, packaging) = discarded(&format!("# TASK-X-001\n\n{body}"));
    assert_eq!((bytes, packaging.lines), (body.clone().into_bytes(), 2));
    let chat = "Checked:\n```\nwc -l src/lib.rs\n```\n\n";
    let (bytes, unwrapped, packaging) = discarded(&format!("{chat}{body}"));
    assert_eq!(bytes, body.as_bytes());
    assert!(!unwrapped && !packaging.wrapper);
    assert_eq!((packaging.lines, packaging.bytes), (5, chat.len()));
}

/// A wrapper after chat must be a pure outer fence, or nothing lands.
#[test]
fn chat_then_a_wrapper_that_does_not_close_last_is_refused() {
    let answer = format!("{CHAT}\n\n```markdown\n{}```\nDone!\n", valid_body());
    assert!(
        refusal(&answer).contains(&format!("(first line: \"{CHAT}\")")),
        "{}",
        refusal(&answer)
    );
}

/// Chat that opens a block and never closes it before the task file: the
/// host cannot say where the task file starts, so it refuses.
#[test]
fn chat_that_leaves_a_block_open_before_the_task_file_is_refused() {
    let answer = format!("```markdown\nA chat line.\n{}```\n", valid_body());
    assert!(
        refusal(&answer).contains("(first line: \"```markdown\")"),
        "{}",
        refusal(&answer)
    );
}
