use super::*;

pub(super) fn build_bound(record: &WorkflowV2CallRecord) -> bool {
    record
        .call
        .options
        .host_command
        .as_ref()
        .is_some_and(|request| {
            crate::command::workflow_host_command_logic::CAPABILITY_LOGIC
                .iter()
                .find(|logic| logic.id == request.command_id)
                .is_some_and(|logic| logic.build_bound)
        })
}

pub(super) fn enrich_build_bound(
    pause: &mut ScriptPauseRecord,
    slots: &[WorkflowV2CallRecord],
    store: &WorkflowV2ResultStore,
) {
    for covered in pause
        .covered
        .iter_mut()
        .filter(|covered| covered.build_bound.is_none())
    {
        if let Some(current) = slots.iter().find(|slot| slot.call.id == covered.call_id) {
            covered.build_bound = Some(build_bound(current));
            continue;
        }
        let history_dir = store.call_history_dir(&covered.call_id);
        let Ok(entries) = std::fs::read_dir(history_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            let Ok(record) = serde_json::from_slice::<WorkflowV2CallRecord>(&bytes) else {
                continue;
            };
            if record.call.id == covered.call_id {
                covered.build_bound = Some(build_bound(&record));
                break;
            }
        }
    }
}
