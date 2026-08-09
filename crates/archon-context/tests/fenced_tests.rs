//! The union of the shapes the five hand-rolled fence strippers between them
//! had to survive, plus the two they all got wrong.

use archon_context::fenced::{FencedBlock, fenced_block_tagged, first_fenced_block, json_payload};

/// The expected `(tag, body, terminated)` of one block, or `None` for "no block".
type Expected = Option<(Option<&'static str>, &'static str, bool)>;

/// `first_fenced_block` over every real-world shape, as `(name, input, expected)`.
#[test]
fn first_fenced_block_over_the_real_world_shapes() {
    let cases: &[(&str, &str, Expected)] = &[
        (
            "fenced with a tag",
            "```json\n{\"a\":1}\n```",
            Some((Some("json"), "{\"a\":1}\n", true)),
        ),
        ("fenced bare", "```\n[1]\n```", Some((None, "[1]\n", true))),
        (
            "yaml tag is reported, not swallowed",
            "```yaml\ntask_id: T\n```\n",
            Some((Some("yaml"), "task_id: T\n", true)),
        ),
        (
            "preamble before the fence",
            "Here you go:\n\n```json\n[{\"n\":\"x\"}]\n```",
            Some((Some("json"), "[{\"n\":\"x\"}]\n", true)),
        ),
        (
            "trailing prose after the fence",
            "```json\n{\"a\":1}\n```\n\nHope that helps!",
            Some((Some("json"), "{\"a\":1}\n", true)),
        ),
        (
            "preamble and trailing prose together",
            "Sure:\n```json\n{\"a\":1}\n```\nAnything else?",
            Some((Some("json"), "{\"a\":1}\n", true)),
        ),
        (
            // A cut-off response still carries the payload; reporting nothing
            // would throw away the only copy of it.
            "unterminated fence keeps what it has",
            "```json\n{\"a\":1}",
            Some((Some("json"), "{\"a\":1}", false)),
        ),
        ("no fence at all", "just prose, no code", None),
        ("empty input", "", None),
        (
            // Every one of the five originals used a bare `find(\"```\")` and
            // truncated here, mid-string.
            "a triple backtick quoted inside a string value",
            "```json\n{\"snippet\": \"wrap it in ``` fences\"}\n```",
            Some((
                Some("json"),
                "{\"snippet\": \"wrap it in ``` fences\"}\n",
                true,
            )),
        ),
        (
            "an info string keeps only its language word",
            "```json (as requested)\n{}\n```",
            Some((Some("json"), "{}\n", true)),
        ),
        (
            "CRLF line endings",
            "```json\r\n{\"a\":1}\r\n```\r\n",
            Some((Some("json"), "{\"a\":1}\r\n", true)),
        ),
        (
            "an empty block is a block with an empty body",
            "```json\n```",
            Some((Some("json"), "", true)),
        ),
    ];

    for (name, input, expected) in cases {
        let got = first_fenced_block(input);
        let expected = expected.map(|(tag, body, terminated)| FencedBlock {
            tag,
            body,
            terminated,
        });
        assert_eq!(got, expected, "{name}: input {input:?}");
    }
}

/// The tag filter has to skip blocks rather than stop at the first one, or a
/// task file that opens with a shell example loses its metadata.
#[test]
fn a_tagged_block_is_found_past_blocks_with_other_tags() {
    let doc = "# Task\n\n```bash\ncargo test\n```\n\n```yaml\ntask_id: TASK-X\n```\n";

    let yaml = fenced_block_tagged(doc, "yaml").expect("the yaml block is there");
    assert_eq!(yaml.tag, Some("yaml"));
    assert_eq!(yaml.body, "task_id: TASK-X\n");

    // ...while the untagged search still reports the first block, whatever it is.
    assert_eq!(first_fenced_block(doc).unwrap().tag, Some("bash"));
}

#[test]
fn the_tag_filter_is_case_insensitive_and_rejects_bare_fences() {
    assert!(fenced_block_tagged("```JSON\n{}\n```", "json").is_some());
    assert!(
        fenced_block_tagged("```\n{}\n```", "json").is_none(),
        "a bare fence has no tag, so it cannot match one"
    );
    assert!(fenced_block_tagged("```yml\nx: 1\n```", "yaml").is_none());
}

/// A body line that looks like a fence belongs to its block, so the scan has to
/// resume after the close and not inside it.
#[test]
fn scanning_resumes_after_a_skipped_block_not_inside_it() {
    let doc = "```text\n```yaml\nnot really yaml\n```\n\n```yaml\ntask_id: REAL\n```\n";
    let yaml = fenced_block_tagged(doc, "yaml").expect("the second block");
    assert_eq!(yaml.body, "task_id: REAL\n");
}

/// `json_payload` over the same union. It always returns something borrowed
/// from the input — prose with no JSON comes back intact so the KB compiler can
/// keep it as a summary.
#[test]
fn json_payload_over_the_real_world_shapes() {
    let cases: &[(&str, &str, &str)] = &[
        ("bare json passes through", " {\"a\":1} ", "{\"a\":1}"),
        ("fenced with a tag", "```json\n{\"a\":1}\n```", "{\"a\":1}"),
        ("fenced bare", "```\n[1]\n```", "[1]"),
        (
            "preamble before the fence",
            "Here you go:\n\n```json\n[{\"name\":\"x\"}]\n```\n",
            "[{\"name\":\"x\"}]",
        ),
        (
            "trailing prose after the fence",
            "```json\n{\"a\":1}\n```\nHope that helps",
            "{\"a\":1}",
        ),
        ("unterminated fence", "```json\n{\"a\":1}", "{\"a\":1}"),
        (
            "unfenced preamble yields the outermost value",
            "Sure! [{\"name\":\"x\"}] hope that helps",
            "[{\"name\":\"x\"}]",
        ),
        (
            "prose without json is left alone",
            "  just prose  ",
            "just prose",
        ),
        ("empty input", "", ""),
        (
            "a quoted triple backtick does not truncate the payload",
            "```json\n{\"s\": \"use ``` here\"}\n```",
            "{\"s\": \"use ``` here\"}",
        ),
        (
            // CommonMark calls this an inline code span, not a block, so the
            // block scan finds an empty body and the bracket scan recovers it.
            "a single-line fence still yields its value",
            "```json {\"a\":1}```",
            "{\"a\":1}",
        ),
    ];

    for (name, input, expected) in cases {
        assert_eq!(json_payload(input), *expected, "{name}: input {input:?}");
    }
}

/// The payloads the fenced shapes produce must actually parse, which is the
/// only thing the callers care about.
#[test]
fn json_payloads_round_trip_through_serde() {
    for input in [
        "{\"a\":1}",
        "```json\n{\"a\":1}\n```",
        "```\n{\"a\":1}\n```",
        "Here you go:\n\n```json\n{\"a\":1}\n```\n\nDone.",
        "```json\n{\"a\":1}",
        "Sure! {\"a\":1} hope that helps",
    ] {
        let payload = json_payload(input);
        let parsed: serde_json::Value = serde_json::from_str(payload)
            .unwrap_or_else(|e| panic!("{input:?} -> {payload:?}: {e}"));
        assert_eq!(parsed["a"], 1, "{input:?}");
    }
}
