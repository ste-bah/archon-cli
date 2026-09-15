//! Prelude wiring: primitive binding, transport retry, review-finding access.

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

        // Inspect the actual JavaScript assembled for QuickJS. Keeping the
        // binding text in source.rs is insufficient if script_source() stops
        // interpolating it.
        let generated = crate::v2::script::script_source(
            "async function workflow(w) { return w.finalReport('done'); }",
            None,
        );
        let bound: std::collections::BTreeSet<&str> = generated
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
            "prelude exports {missing:?} but the globals block never binds them — an authored script calling these gets 'not defined' at dry-run pre-flight. Add `globalThis.<name> = api.<name>;` in source.rs"
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
    /// Pull one named arrow-function definition out of the prelude by name.
    fn prelude_fn(name: &str) -> String {
        let prelude = super::super::V3_PRIMITIVES_JS;
        let marker = format!("  const {name} = ");
        let start = prelude
            .find(&marker)
            .unwrap_or_else(|| panic!("prelude must define {name}"));
        let end = start
            + prelude[start..]
                .find("\n  };")
                .unwrap_or_else(|| panic!("{name} must end with a closing arrow body"))
            + 5;
        prelude[start..end].to_string()
    }

    fn run_js(driver: &str) -> String {
        let mut script = prelude_fn("reviewFindings");
        script.push('\n');
        script.push_str(driver);
        script.push('\n');
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("review.mjs");
        std::fs::write(&path, script).expect("write driver");
        let out = std::process::Command::new("node")
            .arg(&path)
            .output()
            .expect("node must be available");
        assert!(
            out.status.success(),
            "driver failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// `reviewFindings` returns the host's attachment, from any of the three
    /// places the result view exposes it, and NEVER walks the envelope: a
    /// reply carrying findings in every array the old walk read, but no
    /// attachment, yields nothing. Executes the real prelude function.
    #[test]
    fn review_findings_read_the_host_attachment_and_never_walk() {
        let driver = r#"const attached = { review_findings: { findings: [{ id: "F1" }] } };
const nested = { data: { review_findings: { findings: [{ id: "F2" }] } } };
const viewed = { result: { data: { review_findings: { findings: [{ id: "F3" }] } } } };
const walkable = { data: { findings: [{ id: "nope" }], adversarial_findings: [{ id: "nope" }],
  outcomes: [{ result: { data: { findings: [{ id: "nope" }] } } }] } };
console.log(JSON.stringify([reviewFindings(attached), reviewFindings(nested), reviewFindings(viewed),
  reviewFindings(walkable), reviewFindings(null), reviewFindings({})]));"#;
        assert_eq!(
            run_js(driver),
            r#"[[{"id":"F1"}],[{"id":"F2"}],[{"id":"F3"}],[],[],[]]"#
        );
    }

    /// The rule the prelude must NOT carry: which arrays hold findings, which
    /// fields identify one, and how map and reduce findings merge. Those live
    /// in `v2::review_findings` alone. A prelude that grows a copy of any of
    /// them is the defect this pins -- six live failures came from two copies
    /// drifting, and nothing in the build noticed until now.
    #[test]
    fn the_prelude_carries_no_copy_of_the_finding_rules() {
        let prelude = super::super::V3_PRIMITIVES_JS;
        for banned in [
            "\"adversarial_findings\", \"uncovered_requirements\"",
            "\"requirement_id\"",
            "const findingIdentities",
            "const attributedMapFindings",
            "const mergeMapAndReduceFindings",
            "const reattributeFindings",
            "const stampTaskIds",
            "const taskIdsOfOutcome",
        ] {
            assert!(
                !prelude.contains(banned),
                "the prelude must not re-derive a finding rule the host owns; found `{banned}`"
            );
        }
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

    /// `reviewMapReduce` must hand the reduce the HOST's attributed map
    /// findings and return the HOST's merged set -- never a set it derived.
    #[test]
    fn review_map_reduce_reads_map_and_reduce_findings_from_the_host() {
        let start = offset_of("  const reviewMapReduce = ");
        let body = &prelude()[start..start + prelude()[start..].find("\n  };").expect("fn end")];

        assert!(
            body.contains("const mapFindings = reviewFindings(map);"),
            "the reduce must receive the host-attributed map findings: {body}"
        );
        assert!(
            body.contains("return reviewFindings(reduce);"),
            "the review must return the host's merged attachment: {body}"
        );
        // `itemTaskIdsPath` is a contract field and may appear; the TABLE may not.
        assert!(
            !body.contains("const itemTaskIds") && !body.contains("itemTaskIds["),
            "no script-side item table: the host attributes from the branch input it built"
        );
    }

    /// Success must be evaluated before any guard that can `continue` or
    /// re-dispatch. Asserted on ORDER in the real loop, because the behavioural
    /// test replays the ordering in its own driver and cannot see this.
    #[test]
    fn the_success_break_precedes_the_transport_guards_in_the_real_loop() {
        let loop_start = offset_of("      for (let round = 1; round <= maxRounds;");
        let body = &prelude()
            [loop_start..loop_start + prelude()[loop_start..].find("\n      }").expect("loop end")];

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
        let body = &prelude()
            [loop_start..loop_start + prelude()[loop_start..].find("\n      }").expect("loop end")];

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

#[test]
fn the_host_owns_the_result_predicates_a_script_would_otherwise_reinvent() {
    // A hand-rolled predicate that disagrees with the host does not fail a run,
    // it loops it: one live run spent every remediation round redoing work that
    // was already complete because its own no-op check and the host's differed.
    let primitives = super::super::v3_prelude::V3_PRIMITIVES_JS;
    // `reviewFindings` joins them: the accounting the host checks must be built
    // from the host's own walk, so a script that hand-rolls extraction reports a
    // subset and the run is refused for dropping findings it never collected.
    for name in ["accepted", "usable", "outcomesOf", "reviewFindings"] {
        assert!(
            primitives.contains(&format!("const {name} =")),
            "the prelude must define {name}"
        );
        // Asserted per name rather than by pinning the whole export tail: that
        // literal broke the moment a predicate was added, which is exactly when
        // the assertion should have kept passing.
        assert!(
            primitives.contains(&format!("{name},")) || primitives.contains(&format!("{name} }}")),
            "the prelude must export {name} with the other primitives"
        );
    }

    let installed = super::super::source::script_source("async function workflow(w) {}", None);
    for name in ["accepted", "usable", "outcomesOf"] {
        assert!(
            installed.contains(&format!("globalThis.{name} = api.{name};")),
            "{name} must be installed as a global beside agent/agents"
        );
    }

    let brief = super::super::v3_author_a::V3_PRIMITIVE_REFERENCE;
    assert!(
        brief.contains("DO NOT WRITE YOUR OWN RESULT PREDICATES"),
        "the brief must point the author at them"
    );
}

/// Issue-19: the reference must FORCE the prelude predicates, not merely
/// mention them. Its example used `isAccepted(impl) && isAccepted(check)` and
/// listed `isAccepted(env) -> env.status === ...` under "your own small
/// helpers", so every authored script wrote one — and the live one required
/// top-level `files_changed`/`commands_run` the envelope did not carry.
#[test]
fn the_reference_forces_the_prelude_status_predicates() {
    let brief = super::super::v3_author_a::V3_PRIMITIVE_REFERENCE;
    assert!(
        brief.contains("every status predicate MUST be\n  `accepted(env)` or `usable(env)`"),
        "the rule must be a MUST"
    );
    assert!(brief.contains("MUST NOT define its own"), "and a MUST NOT");
    assert!(
        brief.contains("if (usable(impl) && accepted(check)) acceptedTaskIds.push(t.id)"),
        "the correct example is given in the rule and used by the example loop"
    );
    assert!(
        brief.contains("(!usable(impl) || !accepted(check))"),
        "the remediation loop condition uses the prelude predicates"
    );
    assert!(
        !brief.contains("isAccepted("),
        "no example line calls a hand-rolled predicate: {}",
        brief
            .lines()
            .find(|l| l.contains("isAccepted("))
            .unwrap_or_default()
    );
    assert!(
        !brief.contains("isAccepted(env) ->"),
        "the helper list no longer tells the author to define one"
    );
    // The envelope description names the top-level mirrors and where the
    // full records live.
    assert!(brief.contains("COMPACT MIRRORS of `result.*`"));
    assert!(brief.contains("`commands_run` is { command, status }"));
    // The pre-flight consequence is stated so the planner knows the lint exists.
    assert!(brief.contains("The dry-run pre-flight rejects a script whose own"));
}
