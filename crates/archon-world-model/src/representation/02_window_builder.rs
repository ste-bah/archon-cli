pub struct TraceWindowBuilder<'a> {
    rows: Vec<WorldTraceRow>,
    /// Fills each window's and action's `embedding`.
    ///
    /// Lives here rather than in `jepa/` on purpose: the encoder path is gated
    /// to stay free of embedding adapters, so the dependency has to point
    /// outward. Without an adapter every window is built with `embedding:
    /// None`, and the encoder falls back to hashing excerpt text into
    /// `latent_dim` buckets.
    embedder: Option<&'a dyn WorldEmbeddingAdapter>,
}

impl<'a> TraceWindowBuilder<'a> {
    pub fn new(rows: &[WorldTraceRow]) -> Self {
        let mut rows = rows.to_vec();
        rows.sort_by(|left, right| {
            left.session_id
                .cmp(&right.session_id)
                .then_with(|| left.created_at.cmp(&right.created_at))
                .then_with(|| left.row_id.cmp(&right.row_id))
        });
        Self {
            rows,
            embedder: None,
        }
    }

    /// Build windows with dense embeddings instead of hashed excerpt text.
    pub fn with_embedding_adapter(mut self, embedder: &'a dyn WorldEmbeddingAdapter) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Embed the joined row text of a window.
    ///
    /// Returns `None` on any embedding failure rather than propagating: an
    /// unavailable provider must degrade to the hashed fallback, not abort a
    /// training run that would otherwise succeed.
    fn embed_rows(&self, rows: &[WorldTraceRow]) -> Option<Vec<f32>> {
        let embedder = self.embedder?;
        if rows.is_empty() {
            return None;
        }
        let text = rows.iter().map(row_text).collect::<Vec<_>>().join("\n");
        // Keyed on the window's span so the adapter's cache can distinguish two
        // windows that share an anchor but cover different rows.
        let source_hash = format!(
            "window:{}:{}:{}",
            rows[0].session_id,
            rows[0].row_id,
            rows.len()
        );
        embed_text(embedder, source_hash, &text)
    }

    /// `TraceAction::from_row`, plus the action's own embedding when an adapter
    /// is configured. The action is embedded separately from its window: the
    /// encoder scores the two independently, and reusing the window vector
    /// would make every action in a window look identical.
    fn action_from_row(&self, row: &WorldTraceRow) -> TraceAction {
        let mut action = TraceAction::from_row(row);
        action.embedding = self.embedder.and_then(|embedder| {
            embed_text(
                embedder,
                format!("action:{}", action.action_ref),
                &action_embedding_text(&action),
            )
        });
        action
    }

    pub fn context_window(&self, anchor_row_id: &str, context_rows: usize) -> Result<TraceWindow> {
        let index = self.index_of(anchor_row_id)?;
        self.context_window_at(index, context_rows)
    }

    pub fn prior_context_window(
        &self,
        anchor_row_id: &str,
        context_rows: usize,
    ) -> Result<TraceWindow> {
        let index = self.index_of(anchor_row_id)?;
        self.prior_context_window_at(index, context_rows)
    }

    pub fn target_window(
        &self,
        anchor_row_id: &str,
        target_rows: usize,
        horizon: usize,
    ) -> Result<TraceWindow> {
        let index = self.index_of(anchor_row_id)?;
        self.target_window_at(index, target_rows, horizon)
    }

    pub fn adjacent_transitions(
        &self,
        context_rows: usize,
        target_rows: usize,
        horizon: usize,
    ) -> Result<Vec<TraceTransition>> {
        if context_rows == 0 || target_rows == 0 || horizon == 0 {
            bail!("trace window sizes and horizon must be greater than zero");
        }

        let mut transitions = Vec::new();
        for index in 0..self.rows.len().saturating_sub(horizon) {
            let current = &self.rows[index];
            let target_index = index + horizon;
            let target = &self.rows[target_index];
            if current.session_id.as_str() != target.session_id.as_str() {
                continue;
            }

            transitions.push(TraceTransition {
                context: self.prior_context_window_at(index, context_rows)?,
                action: self.action_from_row(current),
                target: self.target_window_at(index, target_rows, horizon)?,
                labels: target.labels.clone(),
            });
        }

        Ok(transitions)
    }

    fn index_of(&self, row_id: &str) -> Result<usize> {
        self.rows
            .iter()
            .position(|row| row.row_id == row_id)
            .ok_or_else(|| anyhow::anyhow!("trace row not found: {row_id}"))
    }

    fn prior_context_window_at(&self, index: usize, context_rows: usize) -> Result<TraceWindow> {
        if context_rows == 0 {
            bail!("context_rows must be greater than zero");
        }
        let (session_start, _) = self.session_bounds(index);
        let start = index.saturating_sub(context_rows).max(session_start);
        self.window_from_range_allow_empty(index, start, index, 0)
    }

    fn context_window_at(&self, index: usize, context_rows: usize) -> Result<TraceWindow> {
        if context_rows == 0 {
            bail!("context_rows must be greater than zero");
        }
        let (session_start, _) = self.session_bounds(index);
        let start = (index + 1).saturating_sub(context_rows).max(session_start);
        self.window_from_range(index, start, index + 1, 0)
    }

    fn target_window_at(
        &self,
        index: usize,
        target_rows: usize,
        horizon: usize,
    ) -> Result<TraceWindow> {
        if target_rows == 0 || horizon == 0 {
            bail!("target_rows and horizon must be greater than zero");
        }
        let (_, session_end) = self.session_bounds(index);
        let start = index + horizon;
        if start >= session_end {
            bail!("target window crosses session boundary");
        }
        let end = (start + target_rows).min(session_end);
        self.window_from_range(index, start, end, horizon)
    }

    fn window_from_range_allow_empty(
        &self,
        anchor_index: usize,
        start: usize,
        end: usize,
        horizon: usize,
    ) -> Result<TraceWindow> {
        if start > end || end > self.rows.len() {
            bail!("invalid trace window range");
        }
        let anchor = &self.rows[anchor_index];
        let rows = self.rows[start..end].to_vec();
        let embedding = self.embed_rows(&rows);
        Ok(TraceWindow {
            session_id: anchor.session_id.clone(),
            anchor_row_id: anchor.row_id.clone(),
            rows,
            horizon,
            graph_context: graph_context_for_row(&self.rows, anchor),
            embedding,
        })
    }

    fn window_from_range(
        &self,
        anchor_index: usize,
        start: usize,
        end: usize,
        horizon: usize,
    ) -> Result<TraceWindow> {
        if start >= end {
            bail!("invalid trace window range");
        }
        self.window_from_range_allow_empty(anchor_index, start, end, horizon)
    }

    fn session_bounds(&self, index: usize) -> (usize, usize) {
        let session_id = &self.rows[index].session_id;
        let start = (0..=index)
            .rev()
            .find(|candidate| self.rows[*candidate].session_id.as_str() != session_id.as_str())
            .map(|candidate| candidate + 1)
            .unwrap_or(0);
        let end = (index + 1..self.rows.len())
            .find(|candidate| self.rows[*candidate].session_id.as_str() != session_id.as_str())
            .unwrap_or(self.rows.len());
        (start, end)
    }
}
