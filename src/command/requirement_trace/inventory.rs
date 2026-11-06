//! Trace consumes the same source inventory as acceptance coverage.
use archon_knowledge::traceability::Requirement;

pub(super) fn extract_requirements(source: &str) -> Result<Vec<Requirement>, String> {
    let lines = archon_workflow::obligation_ids::obligation_source_lines(source);
    archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(source)
        .into_iter()
        .map(|(id, text)| {
            let line = lines
                .get(&id)
                .copied()
                .ok_or_else(|| format!("source requirement {id:?} has no declaration line"))?;
            Ok(Requirement::from_statement(id, text, line))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trace_uses_the_host_inventory_for_every_authored_style() {
        for id in ["REQ-X-001", "FR-001", "R1", "REQ-1", "US-12", "ITEM-1"] {
            let source = format!("Requirements\n---\n- **{id}**: fail closed\n");
            let rows = extract_requirements(&source).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, id);
            assert_eq!(rows[0].text, "fail closed");
            assert_eq!(rows[0].line, 3);
            assert!(rows[0].is_error_severity());
        }
        let source = "## Work\n| Key | Requirement |\n|---|---|\n| R1 | first |\n\n## Done Definition\n1. second\n";
        let rows = extract_requirements(source).unwrap();
        assert_eq!(rows.iter().map(|row| row.line).collect::<Vec<_>>(), [4]);
    }
}
