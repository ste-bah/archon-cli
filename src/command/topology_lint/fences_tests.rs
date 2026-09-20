use super::*;

/// The body author's reply from run wf-7acf8d4a for TASK-TRADING-002,
/// verbatim as it landed: ```` ```markdown ```` on line 1, the frontmatter's
/// ```` ```yaml ```` on line 2, inner blocks at 84/90 and 98/103, and a bare
/// ```` ``` ```` closing the outer fence on the last line, with no trailing
/// newline.
pub(crate) const TASK_TRADING_002_FENCED: &str =
    include_str!("../../../tests/fixtures/fenced-body/TASK-TRADING-002.md");

const NORMAL_BODY: &str = "# TASK-X-001\n\n```yaml\ntask_id: TASK-X-001\n```\n\n## Files Expected to Change\n\n- `src/lib.rs` — exists (1 lines)\n\n## Focused Tests\n\n```bash\ncargo test -p x\n```\n";

#[test]
fn the_wrapped_trading_body_unwraps_to_the_document_the_author_meant() {
    let inner = unwrap_outer_fence(TASK_TRADING_002_FENCED).expect("outer fence recognised");
    assert!(
        inner.starts_with("```yaml\ntask_id: TASK-TRADING-002\n"),
        "{:?}",
        &inner[..40]
    );
    assert!(
        inner.ends_with("fails closed on `ValidationStatus::Failed`.\n"),
        "{:?}",
        &inner[inner.len() - 60..]
    );
    assert_eq!(
        inner.lines().count(),
        122,
        "lines 2..=123 of the 124-line reply"
    );
    assert_eq!(inner.lines().filter(|line| is_fence_line(line)).count(), 6);
    // The interior is a verbatim slice: the same bytes, minus the two fence lines.
    assert_eq!(
        TASK_TRADING_002_FENCED.len(),
        "```markdown\n".len() + inner.len() + "```".len()
    );
    // Idempotent: the unwrapped document is not itself wrapped.
    assert_eq!(unwrap_outer_fence(inner), None);
}

#[test]
fn a_bare_or_md_opener_is_accepted_and_a_yaml_opener_never_is() {
    let doc =
        |opener: &str| format!("{opener}\n```yaml\ntask_id: T\n```\n\n## Plan\n\nText.\n```\n");
    for opener in ["```", "```md", "```Markdown", "```text", "  ```markdown"] {
        assert_eq!(
            unwrap_outer_fence(&doc(opener)),
            Some("```yaml\ntask_id: T\n```\n\n## Plan\n\nText.\n"),
            "{opener}"
        );
    }
    assert_eq!(
        unwrap_outer_fence(&doc("```yaml")),
        None,
        "the frontmatter is not a wrapper"
    );
    assert_eq!(unwrap_outer_fence(&doc("```yml")), None);
    assert_eq!(
        unwrap_outer_fence(&doc("```markdown extra words")),
        None,
        "not an info string"
    );
}

#[test]
fn a_normal_body_is_left_alone() {
    assert_eq!(unwrap_outer_fence(NORMAL_BODY), None);
    assert_eq!(unwrap_outer_fence("# T\n\nNo fences at all.\n"), None);
}

#[test]
fn a_body_that_is_only_its_frontmatter_fence_is_left_alone() {
    let only_frontmatter = "```yaml\ntask_id: TASK-X-001\ntitle: T\n```\n";
    assert_eq!(unwrap_outer_fence(only_frontmatter), None);
    // Same with blank lines around it: the first non-blank line is still the
    // yaml opener, so rule 1 refuses.
    assert_eq!(
        unwrap_outer_fence("\n\n```yaml\ntask_id: TASK-X-001\n```\n\n"),
        None
    );
}

#[test]
fn an_empty_or_fence_only_document_is_left_alone() {
    for doc in [
        "",
        "\n\n",
        "```",
        "```\n",
        "```\n```",
        "```markdown\n```\n",
        "```\n```yaml\n",
    ] {
        assert_eq!(unwrap_outer_fence(doc), None, "{doc:?}");
    }
}

#[test]
fn a_last_fence_that_closes_an_inner_block_does_not_strip() {
    // The opener is an outer wrapper the author never closed; the final ```
    // belongs to the bash block. Stripping would leave that block open, so
    // the odd interior fence count refuses.
    let unclosed =
        "```markdown\n```yaml\ntask_id: T\n```\n\n## Focused Tests\n\n```bash\ncargo test\n```\n";
    assert_eq!(unwrap_outer_fence(unclosed), None);
    // A body whose last line is an inner closer and whose first line is a
    // heading is simply not wrapped.
    assert_eq!(unwrap_outer_fence(NORMAL_BODY.trim_end()), None);
}

#[test]
fn the_wrapper_must_open_on_the_frontmatter() {
    // A fenced snippet that happens to span the whole file but holds a
    // heading first is not a task body wrapper.
    assert_eq!(
        unwrap_outer_fence("```\n# Not a task\n\nText.\n```\n"),
        None
    );
    // Blank lines inside the wrapper before the frontmatter are fine.
    assert_eq!(
        unwrap_outer_fence("```md\n\n```yaml\ntask_id: T\n```\n\nText.\n\n```\n\n"),
        Some("\n```yaml\ntask_id: T\n```\n\nText.\n\n")
    );
}

#[test]
fn crlf_line_endings_unwrap_to_the_interior_verbatim() {
    let doc = "```markdown\r\n```yaml\r\ntask_id: T\r\n```\r\n\r\nText.\r\n```\r\n";
    assert_eq!(
        unwrap_outer_fence(doc),
        Some("```yaml\r\ntask_id: T\r\n```\r\n\r\nText.\r\n")
    );
}

/// The failure mode, pinned: under the shared toggle, the wrapped reply's
/// `## Files Expected to Change` observations all read as fenced, so no prose
/// lint can see them. Unwrapped, the same lines are prose.
#[test]
fn the_shared_toggle_reads_a_wrapped_body_inside_out() {
    let raw = TASK_TRADING_002_FENCED;
    let observation = "- `crates/archon-trading/src/data_lake.rs` — exists (382 lines)";
    assert!(raw.contains(observation));
    assert!(
        !prose_lines(raw).any(|line| line == observation),
        "the observation line is invisible on the raw reply"
    );
    let kinds: Vec<LineKind> = classified_lines(raw).map(|(kind, _)| kind).collect();
    assert_eq!(kinds[0], LineKind::Fence, "line 1: ```markdown");
    assert_eq!(kinds[1], LineKind::Fence, "line 2: ```yaml");
    assert_eq!(kinds[2], LineKind::Prose, "the frontmatter reads as prose");
    assert_eq!(
        kinds[12],
        LineKind::Fence,
        "line 13: the frontmatter's close"
    );
    assert!(
        kinds[13..83].iter().all(|kind| *kind == LineKind::Fenced),
        "lines 14..=83, the whole prose body up to the first inner block, read as fenced"
    );
    // The inversion runs the other way too: the inner ```bash blocks now
    // read as prose, so the commands are what a prose lint would see.
    assert_eq!(kinds[83], LineKind::Fence, "line 84: ```bash");
    assert!(
        kinds[84..89].iter().all(|kind| *kind == LineKind::Prose),
        "lines 85..=89, the cargo commands, read as prose"
    );
    assert!(
        prose_lines(raw).any(|line| line.starts_with("cargo nextest run -p archon-trading")),
        "a fenced command is visible as prose"
    );
    assert_eq!(kinds[123], LineKind::Fence, "line 124: the outer close");
    let unwrapped = unwrap_outer_fence(raw).unwrap();
    assert!(prose_lines(unwrapped).any(|line| line == observation));
    assert_eq!(
        prose_lines(unwrapped)
            .filter(|line| line.contains("— exists ("))
            .count(),
        6,
        "all six deliverable observations are prose once unwrapped"
    );
}

#[test]
fn the_shared_toggle_matches_the_lints_old_behaviour_on_a_well_formed_body() {
    let prose: Vec<&str> = prose_lines(NORMAL_BODY).collect();
    assert_eq!(
        prose,
        vec![
            "# TASK-X-001",
            "",
            "",
            "## Files Expected to Change",
            "",
            "- `src/lib.rs` — exists (1 lines)",
            "",
            "## Focused Tests",
            "",
        ]
    );
    assert!(is_fence_line("   ```bash"));
    assert!(!is_fence_line("text ```"));
}
