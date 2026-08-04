//! Remediation gate reach and behaviour.

#[cfg(test)]
mod remediation_gate_tests {
    /// The gate's reach, pinned as an invariant instead of a claim.
    ///
    /// `patch_landed` is stamped in ONE place — `run_one_worktree_branch` — so
    /// the gate only bites on worktree writes. That is currently total coverage
    /// for its only consumer, because every write the v3 prelude can request is
    /// `write: "worktree"`, and the host does not silently downgrade: a worktree
    /// request with no workspace-boundary support is an error, never a fall back
    /// to serial or coordinated.
    ///
    /// If someone adds a coordinated or serial write to the prelude, that write
    /// gets no marker, `landedNothing` reads absence as "run the check", and the
    /// gate quietly stops applying to it — real, tested, and not reaching, which
    /// is this project's signature failure. This test is the tripwire: it fails
    /// the moment the prelude can emit a write the stamp does not cover.
    #[test]
    fn every_write_the_prelude_requests_is_a_worktree_write() {
        let prelude = super::super::V3_PRIMITIVES_JS;
        let modes: Vec<&str> = prelude
            .match_indices("write: \"")
            .map(|(offset, _)| {
                let rest = &prelude[offset + "write: \"".len()..];
                &rest[..rest.find('"').expect("unterminated write mode literal")]
            })
            .collect();
        assert!(
            !modes.is_empty(),
            "failed to parse any write mode from the prelude; the guard would pass vacuously"
        );
        assert!(
            modes.iter().all(|mode| *mode == "worktree"),
            "the patch_landed marker is only stamped on the worktree write path, but the prelude \
             requests {modes:?}. Either stamp the new path in workflow_live_v2_write_worktree_branch.rs's \
             sibling for that mode, or the remediation gate silently stops applying to it."
        );
    }

    fn run_gate_js(driver: &str) -> String {
        let prelude = super::super::V3_PRIMITIVES_JS;
        let start = prelude
            .find("    const landedNothing = ")
            .expect("landedNothing must exist");
        let end = start + prelude[start..].find("\n    };").expect("fn end") + 7;
        let script = format!("{}\n{driver}\n", &prelude[start..end]);
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("gate.mjs");
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

    /// One contiguous slice of the envelope helpers, so a single-expression
    /// arrow cannot be swallowed by a neighbour's terminator and re-emitted —
    /// which is exactly what a per-name extractor did here, producing a
    /// duplicate `const` that only `node` caught.
    fn envelope_helpers_js() -> String {
        let prelude = super::super::V3_PRIMITIVES_JS;
        let start = prelude
            .find("    const acceptedEnvelope = ")
            .expect("acceptedEnvelope must exist");
        let end = prelude
            .find("    const blocked = ")
            .expect("blocked must follow the envelope helpers");
        assert!(start < end, "envelope helpers must precede `blocked`");
        prelude[start..end].to_string()
    }

    fn run_helpers_js(driver: &str) -> String {
        let script = format!("{}\n{driver}\n", envelope_helpers_js());
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("outcome.mjs");
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

    /// A refutation and a failure must not share a bucket, and the agent's own
    /// evidence must travel with the record.
    ///
    /// The remediation prompt invites refutation explicitly. An agent that
    /// complies produces the most valuable output in the loop and still ends in
    /// `unresolved`; with dozens of findings, a human triaging the list cannot
    /// tell it apart from an agent that tried and could not. Distinguished by a
    /// typed `outcome`, and the evidence is carried VERBATIM rather than
    /// summarised, because the summary is the part that has no triage value.
    #[test]
    fn a_refuted_finding_is_recorded_distinctly_from_a_failed_fix() {
        let driver = r#"
const classify = (fix, check) => {
  if (acceptedEnvelope(fix) && acceptedEnvelope(check)) return { outcome: "resolved" };
  if (check) return { outcome: "unverified" };
  if (acceptedEnvelope(fix)) return { outcome: "refuted", evidence: verbatimEvidence(fix) };
  return { outcome: "failed", evidence: verbatimEvidence(fix) };
};
const refuted = { status: "noop", summary: "finding F1 is wrong: registry writes ARE atomic",
  evidence: [{ kind: "proof", summary: "data_store.rs:212 write-temp-then-rename" }],
  commands_run: [{ command: "cargo test registry_atomic", status: "succeeded" }] };
const failed = { status: "failed", summary: "size policy: openbb.rs 501 > 500" };
const r = classify(refuted, null);
const f = classify(failed, null);
console.log(JSON.stringify({
  refuted: r.outcome,
  failed: f.outcome,
  keeps_refutation_text: r.evidence.indexOf("write-temp-then-rename") >= 0,
  keeps_refutation_commands: r.evidence.indexOf("registry_atomic") >= 0,
  keeps_failure_text: f.evidence.indexOf("501 > 500") >= 0,
  resolved: classify({status:"accepted"}, {status:"accepted"}).outcome,
  unverified: classify({status:"accepted"}, {status:"rejected"}).outcome
}));"#;
        assert_eq!(
            run_helpers_js(driver),
            r#"{"refuted":"refuted","failed":"failed","keeps_refutation_text":true,"keeps_refutation_commands":true,"keeps_failure_text":true,"resolved":"resolved","unverified":"unverified"}"#
        );
    }

    /// Two accepted halves must END the task, whatever the prose says.
    ///
    /// Replays the round loop in its committed order. The old order put both
    /// transport guards ahead of the success break, and `continue` restarts the
    /// round without reaching it — so an accepted fix AND an accepted check were
    /// discarded unread whenever the substring classifier matched something an
    /// agent merely mentioned. That re-ran TDL-041's round 2 after both halves
    /// had passed.
    ///
    /// Asserted on CALL SEQUENCE, not on a boolean: the defect was a wasted
    /// re-dispatch, so the only proof that matters is that the second dispatch
    /// never happens.
    #[test]
    fn two_accepted_halves_end_the_round_before_any_transport_guard_runs() {
        let driver = r#"
const run = (fixQ, checkQ) => {
  const calls = []; let round = 1, retries = 0, fix = null, check = null;
  const maxRounds = 2, maxTransportRetries = 2;
  const agent = (k) => { calls.push(k); const q = k === "fix" ? fixQ : checkQ; return q.length > 1 ? q.shift() : q[0]; };
  while (round <= maxRounds) {
    fix = agent("fix");
    if (transportRetryable(fix) && retries < maxTransportRetries) { retries += 1; continue; }
    if (landedNothing(fix)) { check = null; round += 1; continue; }
    check = agent("check");
    if (acceptedEnvelope(fix) && acceptedEnvelope(check)) break;
    if (transportRetryable(check) && retries < maxTransportRetries) { retries += 1; check = agent("check"); }
    if (acceptedEnvelope(fix) && acceptedEnvelope(check)) break;
    round += 1;
  }
  return calls.join(",");
};
const landed = (extra) => Object.assign({ status: "accepted", data: { patch_landed: true } }, extra);
const ok = { status: "accepted", summary: "clean" };
const dead = { status: "failed", summary: "agent transport failed: 520" };
// Both halves accepted, with transport markers sitting in ordinary agent prose.
const fixProse = landed({ summary: "fixed; the flaky suite timed out after 300s once, re-ran clean" });
const checkProse = { status: "accepted", summary: "verified; agent transport failed earlier, retried" };
console.log(JSON.stringify({
  success_is_terminal: run([fixProse], [checkProse]),
  dead_fix_retries:    run([dead, landed({})], [ok]),
  dead_check_reruns:   run([landed({})], [dead, ok]),
  persistent_dead:     run([landed({})], [dead])
}));"#;
        assert_eq!(
            run_helpers_js(driver),
            concat!(
                r#"{"success_is_terminal":"fix,check","#,
                r#""dead_fix_retries":"fix,fix,check","#,
                r#""dead_check_reruns":"fix,check,check","#,
                r#""persistent_dead":"fix,check,check,fix,check,check"}"#
            )
        );
    }

    /// The gate must be driven by the host marker, across every shape of
    /// "nothing changed" — and must stay quiet when the marker is absent.
    ///
    /// The absent case is the load-bearing one. Reading absence as "nothing
    /// landed" skips EVERY verifier: only the worktree write path sets this
    /// marker, so a host predating it, or any other write mode, would silently
    /// disable verification instead of skipping one provably useless call. That
    /// inversion was written, and caught only by executing the loop.
    #[test]
    fn the_verifier_is_suppressed_only_on_an_explicit_host_nothing_landed() {
        let driver = r#"
const cases = {
  rejected:  {"status":"failed","data":{"patch_landed":false}},
  noop:      {"status":"noop","data":{"patch_landed":false}},
  landed:    {"status":"accepted","data":{"patch_landed":true}},
  no_marker: {"status":"accepted"},
  absent_env: null,
  mixed_fanout: {"data":{"outcomes":[
    {"result":{"data":{"patch_landed":false}}},
    {"result":{"data":{"patch_landed":true}}}]}}
};
const out = {};
for (const k of Object.keys(cases)) out[k] = landedNothing(cases[k]);
console.log(JSON.stringify(out));"#;
        // Only the two explicit "false" cases suppress the verifier. A missing
        // marker, a missing envelope, and a fanout where any branch landed work
        // all keep the check running.
        assert_eq!(
            run_gate_js(driver),
            r#"{"rejected":true,"noop":true,"landed":false,"no_marker":false,"absent_env":false,"mixed_fanout":false}"#
        );
    }
}
