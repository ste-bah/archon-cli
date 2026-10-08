use super::*;

/// Issue-367: a landed file holding the author's chat line, a blank line and
/// the whole task file in a ```` ```markdown ```` wrapper. Its frontmatter
/// still parses, but every observation reads as fenced: one finding names the
/// cause, never one per deliverable.
#[test]
fn a_landed_chat_and_wrapper_file_is_one_finding_not_one_per_deliverable() {
    let (_temp, project, tasks, tree) = grounded();
    let inner = body(
        "[]",
        "- `src/lib.rs` — exists (1 line)\n- `src/existing.rs` — exists (3 lines)\n- `src/new.rs` — absent",
    );
    assert!(findings(&tasks, &project, &tree, &inner).is_empty());
    let chat = "All facts verified. Authoring the repaired TASK body now.";
    let landed = format!("{chat}\n\n```markdown\n{inner}```\n");
    let found = findings(&tasks, &project, &tree, &landed);
    assert_eq!(
        found,
        vec![format!(
            "TASK-X-001: the task file starts with text before its frontmatter (first line: \"{chat}\"); return only the task file, starting with its ```yaml frontmatter block"
        )]
    );
    // The set gate's per-body findings: the same one, no claim on top.
    let path = tasks.join("TASK-X-001.md");
    let set = super::super::repository_claims::body_findings(
        &tree,
        &project,
        "TASK-X-001",
        &path,
        &landed,
    );
    assert_eq!(set, found);
    // A pure wrapper is read as the document inside it: no finding at all.
    let wrapped = format!("```markdown\n{inner}```\n");
    assert!(findings(&tasks, &project, &tree, &wrapped).is_empty());
}
