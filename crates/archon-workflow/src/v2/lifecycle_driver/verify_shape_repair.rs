// The bounded shape-repair loop for a verification retry inventory, and the
// identical-cause escape hatch that keeps it from spending its whole budget on
// a rejection that nothing about the next attempt could change.
//
// Child of `verify.rs`: the loop is only entered from the verification repair
// path and reads the same `repair_inventory`/`verification` pair, so it moved
// as a unit rather than becoming a policy function that would have to be
// handed all of that state anyway.

use std::collections::BTreeSet;

use super::*;

/// Recorded on the escalation so a reader can tell WHY the route was taken
/// apart from a reducer that asked for it — the same way the repeated-gap
/// escalation states "same residual gap reproduced across two retry
/// generations".
pub(super) const REPEATED_REJECTION_ROUTE_REASON: &str =
    "identical semantic-preservation violations reproduced across two shape-repair attempts";

impl LifecycleDriver {
    /// Bounded shape repair of a verification retry inventory.
    ///
    /// D74: an attempt is adopted only when it preserves the semantic identity
    /// of the items it reshaped. A rejection feeds its violations back as
    /// `unresolved_issues` so the next attempt sees exactly what it violated.
    ///
    /// That feedback is also what makes the loop self-sustaining: appending
    /// issues guarantees `verification_inventory_ready` stays false, so a
    /// reducer that reproduces the SAME violations can only exhaust the budget
    /// and block the wave. Observed in #163 as
    /// `verification-repair-shape-repair-1-1-{1,2,3}`, all rejected for one
    /// cause, then `blocked-verification-failed-1` — nothing about attempt 3
    /// could have differed from attempt 1.
    ///
    /// So a violation set reproduced across two attempts is treated the way a
    /// residual gap reproduced across two retry generations already is
    /// (`repeated_gap_write_remediation_outcomes`): as a routing fact, not as a
    /// reason to try again.
    pub(crate) async fn run_verification_shape_repair(
        &self,
        mut repair_inventory: serde_json::Value,
        verification: &serde_json::Value,
        allowed_task_ids: &[String],
        (wave_index, repair_attempt): (usize, usize),
        evidence: &mut LifecycleEvidence,
    ) -> crate::WorkflowResult<serde_json::Value> {
        let mut shape_attempt = 1usize;
        let mut previous_violations: Option<BTreeSet<String>> = None;
        while !support::verification_inventory_ready(&repair_inventory)
            && !support::array(repair_inventory.get("unresolved_issues")).is_empty()
            && shape_attempt <= self.max_repair_iterations
        {
            let shape_call_id = format!(
                "verification-repair-shape-repair-{wave_index}-{repair_attempt}-{shape_attempt}"
            );
            let issues = support::array(repair_inventory.get("unresolved_issues"));
            let shape_repair = self
                .reduce(
                    &shape_call_id,
                    serde_json::json!([
                        self.task_universe,
                        repair_inventory,
                        issues,
                        verification,
                        evidence.implementation
                    ]),
                    "reducer",
                    prompts::VERIFICATION_REPAIR_SHAPE_REPAIR_TASK,
                )
                .await?;
            support::record_repair_attempt(
                &mut evidence.repair_attempts,
                &shape_call_id,
                "verification_repair_shape_repair",
                &issues,
                &shape_repair,
            );
            let candidate =
                self.normalized_shape_candidate(&shape_repair, verification, allowed_task_ids);
            let preservation = semantic_preservation::check_items(
                &support::array(repair_inventory.get("items")),
                &support::array(candidate.get("items")),
            );
            if preservation.passed() {
                repair_inventory = candidate;
                shape_attempt += 1;
                continue;
            }
            support::record_repair_attempt(
                &mut evidence.repair_attempts,
                &shape_call_id,
                "semantic_preservation_rejected",
                &semantic_preservation::violation_issues(&preservation.violations),
                &candidate,
            );
            self.record_preservation_rejection(&shape_call_id, &preservation.violations)
                .await?;
            let signature = violation_signature(&preservation.violations);
            if previous_violations.as_ref() == Some(&signature) {
                let call_id = format!(
                    "verification-repair-shape-unsatisfiable-{wave_index}-{repair_attempt}"
                );
                return Ok(self.escalate_unsatisfiable_shape_repair(
                    repair_inventory,
                    &candidate,
                    ShapeEscalation {
                        call_id: &call_id,
                        verification,
                        allowed_task_ids,
                        violations: &preservation.violations,
                    },
                    evidence,
                ));
            }
            previous_violations = Some(signature);
            semantic_preservation::append_preservation_issues(
                &mut repair_inventory,
                &preservation.violations,
            );
            shape_attempt += 1;
        }
        Ok(repair_inventory)
    }

    /// A reducer's shape repair put through the same normalization the inventory
    /// it is replacing went through, so the preservation check compares like
    /// with like rather than raw reducer output against normalized state.
    fn normalized_shape_candidate(
        &self,
        raw: &serde_json::Value,
        verification: &serde_json::Value,
        allowed_task_ids: &[String],
    ) -> serde_json::Value {
        let contract = self.contract();
        let candidate = contract.normalize_inventory(raw);
        let candidate =
            lifecycle_policy::verify_invariants::enforce_retry_invariants(&candidate, verification);
        support::constrain_inventory_tasks(&contract, &candidate, allowed_task_ids)
    }

    /// Route a shape repair whose rejection reproduced verbatim.
    ///
    /// The rejected candidate is re-offered under an explicit
    /// `predicate_unsatisfiable_as_written` declaration, which is the one route
    /// the preservation guard honours for re-authoring `failed_predicate`,
    /// `classification` and `failure_evidence`
    /// (`semantic_preservation::REAUTHORABLE_WITH_ROUTE`). Everything the guard
    /// refused it for otherwise — gap identity, task identity, source binding,
    /// and dropping an item outright — is still refused, and
    /// `predicate_rewrite_inventory` re-stamps `source_residual_gap_ids` from
    /// the failed outcome itself rather than trusting the reducer for it.
    ///
    /// So this converges the loop when the disagreement was about a predicate
    /// the reducer could not satisfy as written, and stops it without adopting
    /// anything when the disagreement was about identity. Either way the loop
    /// ends here: a third attempt against an unchanged input has nothing new to
    /// return.
    fn escalate_unsatisfiable_shape_repair(
        &self,
        repair_inventory: serde_json::Value,
        candidate: &serde_json::Value,
        escalation: ShapeEscalation<'_>,
        evidence: &mut LifecycleEvidence,
    ) -> serde_json::Value {
        let route = unsatisfiable_shape_route(candidate, escalation.violations);
        let adopted = lifecycle_policy::verify_routing::predicate_rewrite_inventory(
            &route,
            escalation.verification,
        )
        .map(|rewritten| {
            self.normalized_shape_candidate(
                &rewritten,
                escalation.verification,
                escalation.allowed_task_ids,
            )
        })
        .filter(|rewritten| {
            semantic_preservation::check_items(
                &support::array(repair_inventory.get("items")),
                &support::array(rewritten.get("items")),
            )
            .passed()
        });
        support::record_repair_attempt(
            &mut evidence.repair_attempts,
            escalation.call_id,
            "verification_repair_shape_unsatisfiable",
            &semantic_preservation::violation_issues(escalation.violations),
            &serde_json::json!({
                "status": "accepted",
                // `record_repair_attempt` keeps `summary` as the attempt's
                // `reason`, so the reason a route was taken survives into the
                // terminal report rather than only into `data`.
                "summary": REPEATED_REJECTION_ROUTE_REASON,
                "data": {
                    "route": "predicate_unsatisfiable_as_written",
                    "route_reason": REPEATED_REJECTION_ROUTE_REASON,
                    "reauthoring_adopted": adopted.is_some(),
                    "preservation_violations": escalation.violations,
                }
            }),
        );
        adopted.unwrap_or(repair_inventory)
    }
}

/// The four borrowed inputs the escalation needs beyond the two inventories it
/// is choosing between. Grouped because they are all read-only context for one
/// decision and passing them individually puts the method over the argument
/// limit for no gain in clarity.
pub(super) struct ShapeEscalation<'a> {
    pub(super) call_id: &'a str,
    pub(super) verification: &'a serde_json::Value,
    pub(super) allowed_task_ids: &'a [String],
    pub(super) violations: &'a [String],
}

/// Two rejections are the same rejection when they name the same violations.
///
/// Compared as a set, not as the produced sequence: the guard's ordering
/// follows the order the reducer returned items in, and a reducer that
/// reshuffles the same broken items has still failed for the same reason.
pub(super) fn violation_signature(violations: &[String]) -> BTreeSet<String> {
    violations.iter().cloned().collect()
}

/// The `predicate_unsatisfiable_as_written` route a repeated shape rejection
/// escalates to.
///
/// Shaped as a repair-plan value rather than as an inventory because that is
/// what `predicate_rewrite_inventory` consumes — the same consumer the outer
/// repair-plan path uses at `verify.rs`, so the route is honoured by exactly
/// one implementation instead of a second one written for this caller.
pub(super) fn unsatisfiable_shape_route(
    candidate: &serde_json::Value,
    violations: &[String],
) -> serde_json::Value {
    let items: Vec<serde_json::Value> = support::array(candidate.get("items"))
        .into_iter()
        .map(|mut item| {
            if let Some(object) = item.as_object_mut() {
                object.insert(
                    "route".to_string(),
                    serde_json::json!("predicate_unsatisfiable_as_written"),
                );
            }
            item
        })
        .collect();
    serde_json::json!({
        "status": "accepted",
        "data": {
            "route": "predicate_unsatisfiable_as_written",
            "route_reason": REPEATED_REJECTION_ROUTE_REASON,
            "preservation_violations": violations,
            "re_authored_items": items,
        }
    })
}

#[cfg(test)]
#[path = "verify_shape_repair_tests.rs"]
mod tests;
