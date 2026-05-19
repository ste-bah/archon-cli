#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::DeterministicHashEmbeddingAdapter;
    use crate::schema::{WorldActionKind, WorldTraceRow};

    #[test]
    fn builder_orders_rows_and_respects_session_boundaries() {
        let first = WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id("a");
        let second = WorldTraceRow::new("s1", WorldActionKind::Verification).with_row_id("b");
        let third = WorldTraceRow::new("s2", WorldActionKind::Retry).with_row_id("c");
        let builder = TraceWindowBuilder::new(&[third, second, first]);

        let transitions = builder.adjacent_transitions(2, 1, 1).unwrap();

        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].context.rows.len(), 1);
        assert_eq!(transitions[0].target.rows[0].row_id, "b");
    }

    #[test]
    fn graph_context_is_computed_at_anchor_row() {
        let mut first = WorldTraceRow::new("s1", WorldActionKind::PlanUpdate).with_row_id("plan");
        first.agent = Some("coder".into());
        let mut second = WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id("tool");
        second.agent = Some("coder".into());
        let builder = TraceWindowBuilder::new(&[first, second]);

        let window = builder.context_window("tool", 2).unwrap();

        assert_eq!(window.anchor_row_id, "tool");
        assert_eq!(window.graph_context.same_agent_prior_count, 1);
    }

    fn make_adapter() -> GenericEmbeddingRepresentationAdapter {
        GenericEmbeddingRepresentationAdapter::new(Box::new(
            DeterministicHashEmbeddingAdapter::new(8).unwrap(),
        ))
    }

    fn make_two_transitions() -> (TraceWindow, TraceWindow, TraceAction) {
        let row1 = WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id("r1");
        let row2 = WorldTraceRow::new("s1", WorldActionKind::Verification).with_row_id("r2");
        let builder = TraceWindowBuilder::new(&[row1.clone(), row2.clone()]);
        let ctx = builder.context_window("r1", 1).unwrap();
        let tgt = builder.target_window("r1", 1, 1).unwrap();
        let action = TraceAction::from_row(&row1);
        (ctx, tgt, action)
    }

    #[test]
    fn encode_state_batch_default_matches_sequential() {
        let adapter = make_adapter();
        let (ctx, _tgt, _action) = make_two_transitions();
        // Build two identical windows to verify ordering is preserved.
        let windows = vec![ctx.clone(), ctx.clone()];

        let batch = adapter.encode_state_batch(&windows).unwrap();
        let seq: Vec<Vec<f32>> = windows
            .iter()
            .map(|w| adapter.encode_state(w).unwrap())
            .collect();

        assert_eq!(batch.len(), 2);
        for (b, s) in batch.iter().zip(seq.iter()) {
            assert_eq!(b, s, "batch and sequential results must be identical");
        }
    }

    #[test]
    fn encode_action_batch_default_matches_sequential() {
        let adapter = make_adapter();
        let (_ctx, _tgt, action) = make_two_transitions();
        let actions = vec![action.clone(), action.clone()];

        let batch = adapter.encode_action_batch(&actions).unwrap();
        let seq: Vec<Vec<f32>> = actions
            .iter()
            .map(|a| adapter.encode_action(a).unwrap())
            .collect();

        assert_eq!(batch.len(), 2);
        for (b, s) in batch.iter().zip(seq.iter()) {
            assert_eq!(b, s);
        }
    }

    #[test]
    fn encode_target_batch_default_matches_sequential() {
        let adapter = make_adapter();
        let (_ctx, tgt, _action) = make_two_transitions();
        let windows = vec![tgt.clone(), tgt.clone()];

        let batch = adapter.encode_target_batch(&windows).unwrap();
        let seq: Vec<Vec<f32>> = windows
            .iter()
            .map(|w| adapter.encode_target(w).unwrap())
            .collect();

        assert_eq!(batch.len(), 2);
        for (b, s) in batch.iter().zip(seq.iter()) {
            assert_eq!(b, s);
        }
    }
}
