use super::{
    acceptance_ids, bullet_requirement_entries, exact_req_or_ac_pattern, is_acceptance_id,
    table_obligation_rows,
};
use std::collections::BTreeSet;

/// Requirements with the established authoring route, including table rows.
pub fn requirement_row_ids(prd: &str) -> BTreeSet<String> {
    let acceptance = acceptance_ids(prd);
    bullet_requirement_entries(prd)
        .into_iter()
        .chain(table_obligation_rows(prd))
        .map(|(id, _)| id)
        .filter(|id| is_exact_requirement_id(id) && !acceptance.contains(id))
        .collect()
}

/// Whether an identifier has the established requirement shape.
pub fn is_exact_requirement_id(id: &str) -> bool {
    exact_req_or_ac_pattern().is_match(id) && !is_acceptance_id(id)
}
