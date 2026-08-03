//! Inventory-item predicates the wave scheduler and the noop router share.
//!
//! Split out of the binary's `workflow_live_v2_lifecycle_waves.rs` so the noop
//! routing tests, which assert that a host-demoted noop keeps its
//! implementation pin, can travel with the policy they cover. The driver still
//! owns the wave loop; these are the value-level predicates it consults.

use crate::generated_lifecycle_support as support;
use crate::generated_lifecycle_support::LifecycleContract;

pub fn item_has_write_ownership(item: &serde_json::Value) -> bool {
    support::present(item.get("target_files"))
        || support::present(item.get("artifact_requirements"))
}

pub fn preserve_host_pinned_implementation(
    contract: &LifecycleContract<'_>,
    inventory: &serde_json::Value,
    noop_reclassified_ids: &std::collections::BTreeSet<String>,
) -> serde_json::Value {
    let mut object = inventory.as_object().cloned().unwrap_or_default();
    object.insert(
        "items".to_string(),
        serde_json::Value::Array(preserve_host_pinned_items(
            contract,
            support::array(inventory.get("items")),
            noop_reclassified_ids,
        )),
    );
    contract.normalize_inventory(&serde_json::Value::Object(object))
}

pub fn preserve_host_pinned_items(
    contract: &LifecycleContract<'_>,
    items: Vec<serde_json::Value>,
    noop_reclassified_ids: &std::collections::BTreeSet<String>,
) -> Vec<serde_json::Value> {
    items
        .into_iter()
        .map(|mut item| {
            if contract
                .canonical_ids_for(&item)
                .iter()
                .any(|id| noop_reclassified_ids.contains(id))
                && let Some(object) = item.as_object_mut()
            {
                object.insert(
                    "work_type".to_string(),
                    serde_json::Value::String("implementation".to_string()),
                );
            }
            item
        })
        .collect()
}
