//! Prelude wiring: primitive binding, transport retry, findings extraction.

#[cfg(test)]
mod primitive_binding_tests {
    /// Every primitive the prelude exports must also be bound as a global.
    ///
    /// Authored (v3) scripts call these bare — `remediateFindings(...)`, not
    /// `api.remediateFindings(...)` — so a primitive that is exported from
    /// `__archonPrimitives` but missing from the globals block does not exist as
    /// far as the script is concerned. That shipped once: the findings-loop
    /// primitive was written, wired into the author reference, and passed every
    /// unit test, then killed a live run at dry-run pre-flight with
    /// `remediateFindings is not defined`.
    ///
    /// It is the same failure as a verifier that is never invoked and a
    /// primitive the validator forbids: the code is correct and unreachable.
    /// Comparing the two lists is cheap; discovering it live is not.
    #[test]
    fn every_exported_primitive_is_bound_as_a_global() {
        let prelude = super::super::V3_PRIMITIVES_JS;
        let frozen = prelude
            .rsplit_once("Object.freeze({")
            .and_then(|(_, tail)| tail.split_once("})"))
            .map(|(inner, _)| inner)
            .expect("prelude must end by freezing its primitive object");
        let exported: std::collections::BTreeSet<&str> = frozen
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();

        // The globals block moved into a sibling part in the file-size split.
        let helpers = [
            include_str!("workflow_live_v2_script_helpers.rs"),
            include_str!("workflow_live_v2_script_helpers_a.rs"),
            include_str!("workflow_live_v2_script_helpers_b.rs"),
        ]
        .concat();
        let bound: std::collections::BTreeSet<&str> = helpers
            .lines()
            .filter_map(|line| line.trim().strip_prefix("globalThis."))
            .filter_map(|rest| rest.split_once(" = api."))
            .map(|(name, _)| name)
            .collect();

        // Guard the guard: if either parse silently yielded nothing, the
        // difference below would be empty and this test would pass vacuously.
        assert!(
            exported.contains("remediateFindings") && exported.contains("agent"),
            "failed to parse the prelude's exported primitives: {exported:?}"
        );
        assert!(
            bound.contains("agent") && bound.contains("coverageAudit"),
            "failed to parse the globals block: {bound:?}"
        );

        let missing: Vec<&str> = exported.difference(&bound).copied().collect();
        assert!(
            missing.is_empty(),
            "prelude exports {missing:?} but the globals block never binds them — an authored script calling these gets 'not defined' at dry-run pre-flight. Add `globalThis.<name> = api.<name>;` in workflow_live_v2_script_helpers.rs"
        );
    }
}

#[cfg(test)]
mod transport_retry_tests {
    /// A provider failure says nothing about the work, so it must not spend a
    /// remediation round.
    ///
    /// Observed live in one run: a 520 on a fix cost TDL-020 half its budget
    /// without a single real attempt, and a 1200s timeout on a verifier left
    /// TDL-070's ACCEPTED patch recorded as unresolved because its checker
    /// died. The second is the worse failure — correct work discarded.
    ///
    /// The host already draws this line: is_write_branch_validation_error
    /// excludes "agent transport failed" so it is not a contract violation.
    /// Asserted against the JS source because the loop is prelude text.
    #[test]
    fn transport_failures_do_not_consume_a_remediation_round() {
        let prelude = super::super::V3_PRIMITIVES_JS;
        let start = prelude
            .find("const transportFailure =")
            .expect("transportFailure must exist");
        // Slice to the function's actual end, not a fixed window. A magic
        // width made this test report failure on correct code twice: the
        // instrument could not reach what it was asked to check.
        let body = &prelude[start..start + prelude[start..].find("\n    };").expect("fn end")];
        // Typed enum first, prose only where the enum cannot exist.
        assert!(
            body.contains(r#""failure_kind":"execution""#),
            "must prefer the host's typed failure kind: {body}"
        );
        assert!(
            body.contains("cancelled"),
            "a deliberate stop must not be refunded as a provider failure: {body}"
        );
        // A wholesale call failure has no branch outcomes and so no
        // failure_kind; the 520 that cost a round was exactly that shape.
        assert!(body.contains("agent transport failed"), "{body}");
        assert!(body.contains("timed out after"), "{body}");

        let loop_start = prelude
            .find("for (let round = 1; round <= maxRounds;")
            .expect("remediation loop must exist");
        let loop_body = &prelude
            [loop_start..loop_start + prelude[loop_start..].find("\n      }").expect("loop end")];
        // The round counter must advance in the BODY, not the for-header, or a
        // transport `continue` would still spend the round.
        assert!(
            !loop_body.contains("maxRounds; round += 1"),
            "round must not auto-increment: a transport retry would consume it"
        );
        assert!(loop_body.contains("round += 1"), "round must still advance");
        assert!(
            loop_body.contains("transportRetries < maxTransportRetries"),
            "transport retries must be bounded so an outage cannot spin: {loop_body}"
        );
    }
}

#[cfg(test)]
mod findings_extraction_tests {
    /// The prelude's findingsFrom must read a FANOUT envelope, not just a
    /// single-agent one. A map is a fanout: its envelope carries no
    /// data.findings, only per-branch outcomes. Reading the top level alone
    /// returned [] for every map, so reduces received nothing and the mandatory
    /// review reported clean while real findings sat unread in the branches.
    ///
    /// Asserted against the JS source because the helper is prelude text, not
    /// Rust: the shape it must traverse is `data.outcomes[i].result.data.findings`.
    #[test]
    fn findings_extraction_traverses_fanout_branch_outcomes() {
        let prelude = super::super::V3_PRIMITIVES_JS;
        let start = prelude
            .find("const findingsFrom =")
            .expect("findingsFrom must exist");
        let body = &prelude[start..start + 900.min(prelude.len() - start)];
        assert!(
            body.contains("outcomes"),
            "findingsFrom must consider fanout branch outcomes: {body}"
        );
        assert!(
            body.contains("outcome.result.data.findings")
                || body.contains("outcome && outcome.result"),
            "findingsFrom must read each branch outcome's own findings: {body}"
        );
    }
}

/// Review findings must carry the task they belong to, or remediation drops them.
///
/// Executes the REAL prelude JS. A source assertion cannot catch this class of
/// bug: the code that lost the ids was syntactically fine and read correctly —
/// it simply never wrote the field, and the loss was invisible until the
/// findings reached `findingsByTask` and every one landed in `unassigned`.
/// The helpers must actually be WIRED IN, not merely correct.
///
/// The behavioural tests in the sibling modules execute the real prelude
/// helpers, but they call them directly and replay the round loop in their own
/// driver. That proves the logic and says nothing about the call sites — delete
/// every use of `attributedMapFindings` from `reviewMapReduce`, or move the
/// success break back below the transport guards, and all of them still pass.
///
/// Found by sabotage: removing the attribution call sites reddened NOTHING.
/// Correct, tested, and unreachable is this project's signature failure, and it
/// had reproduced inside the suite written to catch it. These assertions pin the
/// wiring; the behavioural tests pin the behaviour. Neither substitutes.
#[cfg(test)]
mod prelude_wiring_tests {
    fn prelude() -> &'static str {
        super::super::V3_PRIMITIVES_JS
    }

    fn offset_of(needle: &str) -> usize {
        prelude()
            .find(needle)
            .unwrap_or_else(|| panic!("prelude must contain `{needle}`"))
    }

    /// `reviewMapReduce` must collect findings through the attributing reader
    /// and repair the reduce output, never through the bare `findingsFrom`.
    #[test]
    fn review_map_reduce_collects_findings_through_the_attributing_reader() {
        let start = offset_of("  const reviewMapReduce = ");
        let body = &prelude()[start..start + prelude()[start..].find("\n  };").expect("fn end")];

        assert!(
            body.contains("attributedMapFindings(map, itemTaskIds)"),
            "reviewMapReduce must stamp task ids as it collects the map shards: {body}"
        );
        assert!(
            body.contains("reattributeFindings(findingsFrom(reduce)"),
            "reviewMapReduce must repair attribution the reduce dropped: {body}"
        );
        assert!(
            !body.contains("{ findings: findingsFrom(map) }"),
            "the reduce must receive STAMPED findings; passing findingsFrom(map) directly is the \
             original defect — 43 of 43 adversarial findings reached remediation unattributed"
        );
        assert!(
            body.contains("itemTaskIds[itemId] = taskId"),
            "the item_id -> taskId map must be built while the map items are constructed: {body}"
        );
    }

    /// Success must be evaluated before any guard that can `continue` or
    /// re-dispatch. Asserted on ORDER in the real loop, because the behavioural
    /// test replays the ordering in its own driver and cannot see this.
    #[test]
    fn the_success_break_precedes_the_transport_guards_in_the_real_loop() {
        let loop_start = offset_of("      for (let round = 1; round <= maxRounds;");
        let body = &prelude()[loop_start
            ..loop_start + prelude()[loop_start..].find("\n      }").expect("loop end")];

        let check_dispatch = body
            .find("label: `review-verify-${slug(taskId)}-${round}`")
            .expect("the verifier dispatch must exist");
        let success_break = body
            .find("if (acceptedEnvelope(fix) && acceptedEnvelope(check)) break;")
            .expect("the success break must exist");
        let check_transport_guard = body
            .find("transportRetryable(check)")
            .expect("the check transport guard must exist");
        let landed_gate = body
            .find("if (landedNothing(fix))")
            .expect("the landed-patch gate must exist");
        let fix_transport_guard = body
            .find("transportRetryable(fix)")
            .expect("the fix transport guard must exist");

        assert!(
            success_break < check_transport_guard,
            "two accepted halves must end the round BEFORE any transport guard: `continue` \
             restarts the round without reaching the break, which discarded an accepted pair \
             unread and re-ran TDL-041 round 2 after both halves had passed"
        );
        assert!(
            landed_gate < check_dispatch,
            "the landed-patch gate must precede the verifier dispatch, or the verifier still runs \
             against unchanged code — the defect it exists to stop"
        );
        assert!(
            fix_transport_guard < check_dispatch,
            "a dead fix must be caught before a verifier is spent on it"
        );
    }

    /// The guards must use the success-aware predicate. `transportFailure` is a
    /// substring probe over the whole envelope, so an accepted result that
    /// merely mentions a timeout matches it.
    #[test]
    fn the_round_loop_guards_use_the_success_aware_transport_predicate() {
        let loop_start = offset_of("      for (let round = 1; round <= maxRounds;");
        let body = &prelude()[loop_start
            ..loop_start + prelude()[loop_start..].find("\n      }").expect("loop end")];

        assert!(
            !body.contains("transportFailure(fix)") && !body.contains("transportFailure(check)"),
            "the loop must guard on transportRetryable, not the raw substring probe: an accepted \
             half that merely mentions a timeout in its prose is not a transport failure"
        );
        assert_eq!(
            body.matches("transportRetryable(").count(),
            2,
            "both halves must be guarded by the success-aware predicate"
        );
    }
}
