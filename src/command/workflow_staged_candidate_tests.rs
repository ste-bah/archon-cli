use super::candidate::normalize_task_candidate;

#[test]
fn the_live_chat_and_markdown_wrapper_is_refused() {
    let body = "```yaml\ntask_id: TASK-X-001\ntitle: T\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n";
    let live = format!(
        "All facts verified. Authoring the repaired TASK body now — …\n\n```markdown\n{body}```\n"
    );
    assert_eq!(
        normalize_task_candidate(live.as_bytes().to_vec()).unwrap_err(),
        "the answer has text before/after the task file"
    );
}

#[test]
fn a_pure_outer_fence_keeps_existing_unwrap_behavior() {
    let body = b"```yaml\ntask_id: TASK-X-001\n```\n\nText.\n";
    let wrapped = [b"```markdown\n".as_slice(), body, b"```\n"].concat();
    assert_eq!(
        normalize_task_candidate(wrapped).unwrap(),
        (body.to_vec(), true)
    );
}

#[test]
fn text_after_the_outer_fence_is_refused() {
    let wrapped = b"```markdown\n```yaml\ntask_id: TASK-X-001\n```\n```\nDone!\n".to_vec();
    assert_eq!(
        normalize_task_candidate(wrapped).unwrap_err(),
        "the answer has text before/after the task file"
    );
}

#[test]
fn a_correct_candidate_lands_unchanged_byte_for_byte() {
    let body =
        b"```yaml\ntask_id: TASK-X-001\n```\n\n## Files\n- `src/lib.rs` - exists (1 lines)\n"
            .to_vec();
    assert_eq!(
        normalize_task_candidate(body.clone()).unwrap(),
        (body, false)
    );
}
