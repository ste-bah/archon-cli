use super::*;

pub(crate) fn derive(
    records: &[WorkflowV2CallRecord],
    pause_ids: &[String],
    criteria: &BTreeMap<String, String>,
) -> Result<Derived> {
    derive_with_reply_markers(records, pause_ids, criteria, &BTreeMap::new(), &[])
}
