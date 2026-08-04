use super::*;

const SAMPLE: &str = concat!(
    "## 21. Validation Report Schema\n",
    "\n",
    "```json\n",
    "- REQ-FAKE-999: this bullet is inside a fence and is not a requirement\n",
    "```\n",
    "\n",
    "Rules:\n",
    "\n",
    "- REQ-DL-100: A dataset with any `error` check must have `status=failed`.\n",
    "- REQ-DL-101: A dataset with warnings but no errors must have\n",
    "  `status=degraded`.\n",
    "- REQ-DL-131: Unknown native interval status must fail closed.\n",
    "\n",
    "  This indented line follows a blank line and is not a continuation.\n",
    "- REQ-AHDM-004: `trading-elliott-wave` is secondary and non-authoritative.\n",
);

#[test]
fn extracts_only_column_zero_bullets_outside_fences() {
    let reqs = extract_requirements(SAMPLE);
    let ids: Vec<&str> = reqs.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["REQ-DL-100", "REQ-DL-101", "REQ-DL-131", "REQ-AHDM-004"]
    );
}

#[test]
fn joins_wrapped_continuation_lines() {
    let reqs = extract_requirements(SAMPLE);
    let wrapped = reqs
        .iter()
        .find(|r| r.id == "REQ-DL-101")
        .expect("REQ-DL-101");
    assert!(
        wrapped.text.ends_with("must have `status=degraded`."),
        "continuation not joined: {}",
        wrapped.text
    );
}

#[test]
fn blank_line_ends_the_requirement_paragraph() {
    let reqs = extract_requirements(SAMPLE);
    let req = reqs
        .iter()
        .find(|r| r.id == "REQ-DL-131")
        .expect("REQ-DL-131");
    assert!(
        !req.text.contains("not a continuation"),
        "absorbed text across a blank line: {}",
        req.text
    );
}

#[test]
fn prefix_and_line_are_recorded() {
    let reqs = extract_requirements(SAMPLE);
    let req = reqs.iter().find(|r| r.id == "REQ-AHDM-004").expect("found");
    assert_eq!(req.prefix, "AHDM");
    assert_eq!(
        SAMPLE.lines().nth(req.line - 1).expect("line"),
        "- REQ-AHDM-004: `trading-elliott-wave` is secondary and non-authoritative."
    );
}

#[test]
fn severity_is_derived_only_from_a_recorded_phrase() {
    let reqs = extract_requirements(SAMPLE);
    let by = |id: &str| {
        reqs.iter()
            .find(|r| r.id == id)
            .expect("requirement")
            .clone()
    };

    let hundred = by("REQ-DL-100");
    assert_eq!(hundred.severity, Severity::Error);
    assert_eq!(hundred.severity_evidence.as_deref(), Some("`error`"));

    let one_three_one = by("REQ-DL-131");
    assert_eq!(one_three_one.severity, Severity::Error);
    assert_eq!(
        one_three_one.severity_evidence.as_deref(),
        Some("fail closed")
    );

    // Warnings are not errors, and no phrase matched.
    let one_o_one = by("REQ-DL-101");
    assert_eq!(one_o_one.severity, Severity::Unclassified);
    assert_eq!(one_o_one.severity_evidence, None);
    assert!(!one_o_one.is_error_severity());
}

#[test]
fn entity_cites_the_prd_line_and_is_stable() {
    let reqs = extract_requirements(SAMPLE);
    let req = reqs.first().expect("at least one");
    let a = requirement_entity(req, "PRD.md");
    let b = requirement_entity(req, "PRD.md");
    assert_eq!(a.entity_id, b.entity_id);
    assert_eq!(a.entity_type, REQUIREMENT_ENTITY_TYPE);
    assert_eq!(a.name, req.id);
    assert_eq!(a.source_chunk_id, format!("PRD.md#L{}", req.line));
    // A different PRD path is a different node, not a collision.
    assert_ne!(a.entity_id, requirement_entity(req, "OTHER.md").entity_id);
}
