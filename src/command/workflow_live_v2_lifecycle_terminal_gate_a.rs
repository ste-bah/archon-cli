const MAX_REROUTES_PER_BLOCKED_ID: usize = 2;

#[derive(Default)]
pub(super) struct TerminalGateState {
    pub(super) reroute_counts: BTreeMap<String, usize>,
    pub(super) pending_implementation_items: Vec<Value>,
    pub(super) completed_ids: BTreeSet<String>,
    pub(super) noop_reclassified_ids: BTreeSet<String>,
    pub(super) events: Vec<Value>,
}

#[derive(Debug, PartialEq)]
pub(super) enum TerminalGateDecision {
    Emit,
    Reroute(Value),
}

pub(super) fn decide(
    contract: &LifecycleContract<'_>,
    blocked_id: &str,
    inputs: &Value,
    state: &mut TerminalGateState,
) -> TerminalGateDecision {
    if !blocked_id.starts_with("blocked-") {
        return TerminalGateDecision::Emit;
    }

    let mut kinds = Vec::new();
    let mut work = Vec::new();
    let mut pending_implementation_items = Vec::new();
    let ready_noops = support::array(inputs.get("readyNoopItems"));
    let failed_noops = support::array(inputs.get("failedNoopProof"));
    if !ready_noops.is_empty() && !failed_noops.is_empty() {
        let route = super::workflow_live_v2_lifecycle_noop_routing::route_refuted_noops(
            contract,
            &ready_noops,
            &BTreeSet::new(),
            &failed_noops,
            &state.completed_ids,
            &mut state.noop_reclassified_ids,
        );
        if let super::workflow_live_v2_lifecycle_noop_routing::NoopProofExhaustionRoute::ScheduleImplementation(
            items,
        ) = route
        {
            kinds.push("refuted_noop");
            work.extend(items.iter().cloned());
            pending_implementation_items.extend(items);
        }
    }

    let attempted_retry_ids = attempted_retry_item_ids(inputs);
    let retry_items = keyed_arrays(inputs, &["retry_items", "retryItems"])
        .into_iter()
        .filter(|item| item_id(item).is_none_or(|id| !attempted_retry_ids.contains(id.as_str())))
        .collect::<Vec<_>>();
    if !retry_items.is_empty() {
        kinds.push("retry_items");
        work.extend(retry_items);
    }

    let remediation_items = actionable_inventory_items(inputs);
    if !remediation_items.is_empty() {
        kinds.push("actionable_remediation");
        work.extend(remediation_items);
    }

    if has_retryable_transport_failure(inputs) {
        kinds.push("transport_retry_budget");
    }

    let accepted_unverified = accepted_unverified_implementation_items(contract, inputs, state);
    if !accepted_unverified.is_empty() {
        kinds.push("accepted_unverified_implementation");
        work.extend(accepted_unverified);
    }

    kinds.sort_unstable();
    kinds.dedup();
    if kinds.is_empty() {
        return TerminalGateDecision::Emit;
    }

    let reroute_count = state
        .reroute_counts
        .entry(blocked_id.to_string())
        .or_default();
    if *reroute_count >= MAX_REROUTES_PER_BLOCKED_ID {
        state.events.push(serde_json::json!({
            "kind": "terminal-gate-reroute-exhausted",
            "blocked_id": blocked_id,
            "reroute_count": reroute_count,
            "schedulable_work_kinds": kinds,
            "reason": "bounded terminal-gate reroutes did not clear the schedulable work",
        }));
        return TerminalGateDecision::Emit;
    }
    *reroute_count += 1;
    state
        .pending_implementation_items
        .extend(pending_implementation_items);
    let event = serde_json::json!({
        "kind": "terminal-gate-reroute",
        "blocked_id": blocked_id,
        "reroute_count": reroute_count,
        "schedulable_work_kinds": kinds,
        "work": work.iter().map(compact_work_item).collect::<Vec<_>>(),
    });
    state.events.push(event.clone());
    TerminalGateDecision::Reroute(event)
}

pub(super) fn apply_pending_implementation_items(
    contract: &LifecycleContract<'_>,
    inventory: &Value,
    pending: Vec<Value>,
) -> Value {
    if pending.is_empty() {
        return inventory.clone();
    }
    let mut items = support::array(inventory.get("items"));
    for replacement in pending {
        let replacement_ids = contract
            .canonical_ids_for(&replacement)
            .into_iter()
            .collect::<BTreeSet<_>>();
        if let Some(index) = items.iter().position(|item| {
            contract
                .canonical_ids_for(item)
                .iter()
                .any(|id| replacement_ids.contains(id))
        }) {
            items[index] = replacement;
        } else {
            items.push(replacement);
        }
    }
    let mut object = inventory.as_object().cloned().unwrap_or_default();
    object.insert("items".to_string(), Value::Array(items));
    contract.normalize_inventory(&Value::Object(object))
}

fn attempted_retry_item_ids(value: &Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    collect_attempted_retry_item_ids(value, &mut ids);
    ids
}

fn collect_attempted_retry_item_ids(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            if object.get("kind").and_then(Value::as_str) == Some("verification-triage-retry") {
                for item in support::array(
                    object
                        .get("verificationPlan")
                        .and_then(|plan| plan.get("items")),
                ) {
                    if let Some(id) = item_id(&item) {
                        ids.insert(id);
                    }
                }
            }
            for child in object.values() {
                collect_attempted_retry_item_ids(child, ids);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_attempted_retry_item_ids(child, ids);
            }
        }
        _ => {}
    }
}

fn keyed_arrays(value: &Value, keys: &[&str]) -> Vec<Value> {
    let mut items = Vec::new();
    collect_keyed_arrays(value, keys, &mut items);
    items
}

fn collect_keyed_arrays(value: &Value, keys: &[&str], items: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if keys.contains(&key.as_str()) {
                    items.extend(support::array(Some(child)));
                }
                collect_keyed_arrays(child, keys, items);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_keyed_arrays(child, keys, items);
            }
        }
        _ => {}
    }
}

fn actionable_inventory_items(inputs: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    collect_actionable_inventory_items(inputs, &mut items);
    items
}

fn collect_actionable_inventory_items(value: &Value, items: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            for key in [
                "unscheduledFollowupInventory",
                "actionableRemediationInventory",
                "actionableInventory",
            ] {
                if let Some(inventory) = object.get(key)
                    && support::array(inventory.get("unresolved_issues")).is_empty()
                {
                    items.extend(support::array(inventory.get("items")));
                }
            }
            for child in object.values() {
                collect_actionable_inventory_items(child, items);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_actionable_inventory_items(child, items);
            }
        }
        _ => {}
    }
}

fn has_retryable_transport_failure(value: &Value) -> bool {
    match value {
        Value::Object(object) => {
            let retryable = object
                .get("failure_class")
                .or_else(|| object.get("failure_kind"))
                .and_then(Value::as_str)
                .is_some_and(|kind| kind.to_ascii_lowercase().contains("transport"))
                && object
                    .get("transport_attempts")
                    .and_then(Value::as_u64)
                    .zip(object.get("max_transport_attempts").and_then(Value::as_u64))
                    .is_some_and(|(attempts, max)| attempts < max);
            retryable || object.values().any(has_retryable_transport_failure)
        }
        Value::Array(values) => values.iter().any(has_retryable_transport_failure),
        _ => false,
    }
}

fn accepted_unverified_implementation_items(
    contract: &LifecycleContract<'_>,
    inputs: &Value,
    state: &TerminalGateState,
) -> Vec<Value> {
    let ready_items = support::array(
        inputs
            .get("readyImplementationItems")
            .or_else(|| inputs.get("ready_implementation_items")),
    );
    if ready_items.is_empty() {
        return Vec::new();
    }
    let Some(wave) = inputs
        .get("wave")
        .or_else(|| inputs.get("implementationWave"))
    else {
        return Vec::new();
    };
    let accepted_ids =
        support::matching_accepted_ids(contract, &ready_items, &support::outcomes_of(wave))
            .into_iter()
            .collect::<BTreeSet<_>>();
    if accepted_ids.is_empty() {
        return Vec::new();
    }
    let scheduled_ids = verification_scheduled_ids(contract, inputs);
    ready_items
        .into_iter()
        .filter(|item| {
            contract.canonical_ids_for(item).iter().any(|id| {
                accepted_ids.contains(id)
                    && !state.completed_ids.contains(id)
                    && !scheduled_ids.contains(id)
            })
        })
        .collect()
}

