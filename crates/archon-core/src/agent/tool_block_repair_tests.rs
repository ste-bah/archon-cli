use super::*;

/// The exact shape captured from the live sglang deployment: one valid call, one
/// truncated call, its orphaned closer, and a second valid call.
#[test]
fn the_live_capture_repairs_to_the_call_that_was_split() {
    let names = ["Read", "Read", "", "Grep"];
    let jsons = [
        r#"{"file_path": "/tmp/a.md"}"#,
        r#"{"file_path": "/tmp/b.md""#,
        "}",
        r#"{"pattern": "foo", "path": "/tmp"}"#,
    ];

    let plan = plan_split_tool_repairs(&names, &jsons);

    assert_eq!(
        plan,
        vec![SplitToolRepair {
            target: 1,
            orphan: 2
        }]
    );
}

/// Two split calls in one message. The first repair must make call 0 parse,
/// which removes it as a candidate for the second fragment — without that, both
/// fragments would look ambiguous and both would be refused.
#[test]
fn two_split_calls_in_one_message_stay_unambiguous() {
    let names = ["Read", "", "Read", ""];
    let jsons = [
        "",
        r#"{"file_path": "/tmp/a.md"}"#,
        r#"{"file_path": "/tmp/b.md""#,
        "}",
    ];

    let plan = plan_split_tool_repairs(&names, &jsons);

    assert_eq!(
        plan,
        vec![
            SplitToolRepair {
                target: 0,
                orphan: 1
            },
            SplitToolRepair {
                target: 2,
                orphan: 3
            },
        ]
    );
}

/// GATE 2. A conforming stream has no unnamed block, so the pass must not run
/// at all — this is what keeps Anthropic direct, Vertex and Codex unchanged.
#[test]
fn a_conforming_stream_is_never_touched() {
    let names = ["Read", "Grep", "Write"];
    let jsons = [
        r#"{"file_path": "/a"}"#,
        r#"{"pattern": "x"}"#,
        r#"{"file_path": "/b", "content": "c"}"#,
    ];

    assert!(plan_split_tool_repairs(&names, &jsons).is_empty());
}

/// GATE 3a. A call whose JSON already parses is never a merge target, so a
/// healthy call cannot be corrupted by a stray fragment.
#[test]
fn a_valid_call_is_never_a_merge_target() {
    let names = ["Read", ""];
    let jsons = [r#"{"file_path": "/tmp/a.md"}"#, "}"];

    assert!(
        plan_split_tool_repairs(&names, &jsons).is_empty(),
        "a parsing call must not absorb a fragment"
    );
}

/// GATE 3b. Two truncated calls that would BOTH accept the fragment is a
/// coin flip, so refuse and let the existing error fire.
#[test]
fn an_ambiguous_fragment_is_refused_rather_than_guessed() {
    let names = ["Read", "Read", ""];
    let jsons = [
        r#"{"file_path": "/tmp/a.md""#,
        r#"{"file_path": "/tmp/b.md""#,
        "}",
    ];

    assert!(
        plan_split_tool_repairs(&names, &jsons).is_empty(),
        "two equally plausible targets must not be guessed between"
    );
}

/// A fragment that repairs nothing is left alone — the merge is only committed
/// when the result actually parses.
#[test]
fn a_fragment_that_does_not_complete_anything_is_refused() {
    let names = ["Read", ""];
    let jsons = [r#"{"file_path": "/tmp/b.md""#, "not json at all"];

    assert!(plan_split_tool_repairs(&names, &jsons).is_empty());
}

/// An unnamed block carrying nothing is not evidence of a split call.
#[test]
fn an_empty_orphan_is_ignored() {
    let names = ["Read", ""];
    let jsons = [r#"{"file_path": "/tmp/b.md""#, "   "];

    assert!(plan_split_tool_repairs(&names, &jsons).is_empty());
}

/// The orphan may carry the WHOLE argument object, with the named block left
/// empty — the other half of the same LiteLLM bug.
#[test]
fn an_orphan_carrying_the_entire_object_repairs_an_empty_named_block() {
    let names = ["Read", ""];
    let jsons = ["", r#"{"file_path": "/tmp/a.md"}"#];

    assert_eq!(
        plan_split_tool_repairs(&names, &jsons),
        vec![SplitToolRepair {
            target: 0,
            orphan: 1
        }]
    );
}

/// Nothing to repair when there are no tool calls at all.
#[test]
fn an_empty_message_is_a_no_op() {
    assert!(plan_split_tool_repairs(&[], &[]).is_empty());
}
