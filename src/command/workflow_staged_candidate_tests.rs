//! Issue-367: which bytes of a body author's answer land as the task file.
use super::candidate::{TaskCandidate, normalize_task_candidate};

const FRONTMATTER: &str = "```yaml\ntask_id: TASK-X-001\ntitle: T\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n";

/// A valid task file with every inner block kind a body really carries; a
/// bash block's bare closer directly followed by a yaml example is the shape
/// round 1 took for an outer fence.
fn valid_body() -> String {
    format!(
        "{FRONTMATTER}\n# TASK-X-001\n\n## Plan\n\nRun it:\n\n```bash\ncargo test -p x\n```\n\n```yaml\nkey: value\n```\n\nPlain output:\n\n```\nok\n```\n\n## Files Expected to Change\n\n- `src/lib.rs` - exists (1 lines)\n"
    )
}

fn refusal(candidate: &str) -> String {
    match normalize_task_candidate(candidate.as_bytes().to_vec()) {
        Ok(TaskCandidate {
            bytes, unwrapped, ..
        }) => panic!(
            "landed ({unwrapped}): {:?}",
            String::from_utf8_lossy(&bytes)
        ),
        Err(reason) => reason.to_string(),
    }
}

fn landed(candidate: &[u8]) -> (Vec<u8>, bool) {
    let landed = landed_with_packaging(candidate);
    assert_eq!(landed.packaging, None, "no packaging expected");
    (landed.bytes, landed.unwrapped)
}

fn landed_with_packaging(candidate: &[u8]) -> TaskCandidate {
    let landed = normalize_task_candidate(candidate.to_vec())
        .unwrap_or_else(|reason| panic!("refused: {reason}"));
    assert!(opens_with_frontmatter(&landed.bytes), "{:?}", landed.bytes);
    landed
}

/// The landed-file shape: the first non-blank line (after a BOM) is the
/// frontmatter opener.
fn opens_with_frontmatter(bytes: &[u8]) -> bool {
    let text = std::str::from_utf8(bytes).unwrap();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.lines()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| matches!(line.trim(), "```yaml" | "```yml"))
}

#[test]
fn a_valid_body_with_bash_yaml_and_plain_blocks_lands_byte_identical() {
    let body = valid_body();
    assert_eq!(landed(body.as_bytes()), (body.into_bytes(), false));
}

#[test]
fn leading_blank_lines_are_dropped_and_the_rest_lands_unchanged() {
    let body = valid_body();
    for prefix in ["\n\n", "\r\n\r\n", "  \n\t\n"] {
        let answer = format!("{prefix}{body}");
        assert_eq!(
            landed(answer.as_bytes()),
            (body.clone().into_bytes(), false),
            "{prefix:?}"
        );
    }
}

#[test]
fn a_bom_and_crlf_body_lands_without_the_bom() {
    let crlf = valid_body().replace('\n', "\r\n");
    let answer = format!("\u{feff}\r\n{crlf}");
    assert_eq!(landed(answer.as_bytes()), (crlf.into_bytes(), false));
}

#[test]
fn a_frontmatter_opener_with_trailing_spaces_lands() {
    let body = valid_body().replacen("```yaml\n", "```yaml  \n", 1);
    assert_eq!(landed(body.as_bytes()), (body.clone().into_bytes(), false));
    let yml = valid_body().replacen("```yaml\n", "```yml \n", 1);
    assert_eq!(landed(yml.as_bytes()), (yml.clone().into_bytes(), false));
}

#[test]
fn a_pure_markdown_wrapper_with_an_inner_plain_block_is_stripped() {
    let inner = format!("{FRONTMATTER}\n## Plan\n\n```\nplain\n```\n\nDone.\n");
    for answer in [
        format!("```markdown\n{inner}```\n"),
        format!("\n```md\n\n{inner}```"),
        format!("\u{feff}```markdown\n{inner}```\n\n"),
    ] {
        assert_eq!(
            landed(answer.as_bytes()),
            (inner.clone().into_bytes(), true),
            "{answer:?}"
        );
    }
}

#[test]
fn a_long_first_line_is_quoted_at_most_120_characters() {
    let long = "é".repeat(200);
    let reason = refusal(&format!("{long}\nno task file here\n"));
    assert!(
        reason.contains(&format!("\"{}\"", "é".repeat(120))),
        "{reason}"
    );
    assert!(!reason.contains(&"é".repeat(121)), "{reason}");
}

#[test]
fn a_wrapper_with_text_after_it_or_unpaired_fences_is_refused() {
    let after = format!("```markdown\n{}```\nDone!\n", valid_body());
    assert!(refusal(&after).contains("(first line: \"```markdown\")"));
    let odd = format!("```markdown\n{FRONTMATTER}```\nunclosed\n```\n");
    assert!(refusal(&odd).contains("(first line: \"```markdown\")"));
}

#[test]
fn bytes_that_are_not_utf8_pass_through_for_the_lint_to_report() {
    let bytes = vec![0x60, 0x60, 0x60, 0x0a, 0xff, 0xfe];
    let passed = normalize_task_candidate(bytes.clone()).unwrap();
    assert_eq!((passed.bytes, passed.unwrapped), (bytes, false));
    assert_eq!(passed.packaging, None);
}

#[test]
fn the_landed_shape_check_needs_the_frontmatter_first() {
    use super::candidate::landed_shape;
    assert_eq!(landed_shape(valid_body().as_bytes()), Ok(()));
    assert_eq!(
        landed_shape(b"\xef\xbb\xbf\r\n```yml \r\nx: 1\r\n```\r\n"),
        Ok(())
    );
    let reason = landed_shape(b"# TASK-X-001\n\n```yaml\n```\n").unwrap_err();
    assert!(
        reason.contains("(first line: \"# TASK-X-001\")"),
        "{reason}"
    );
}

#[path = "workflow_staged_packaging_tests.rs"]
mod packaging;
