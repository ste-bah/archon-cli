#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceWindow {
    pub session_id: String,
    pub anchor_row_id: String,
    pub rows: Vec<WorldTraceRow>,
    pub horizon: usize,
    pub graph_context: GraphContextFeatures,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceAction {
    pub action_ref: String,
    pub action_kind: WorldActionKind,
    pub summary: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub scalar_features: ScalarFeatures,
}

impl TraceAction {
    pub fn from_row(row: &WorldTraceRow) -> Self {
        Self {
            action_ref: row.row_id.clone(),
            action_kind: row.action_kind.clone(),
            summary: row.redacted_excerpt.clone().unwrap_or_default(),
            provider: row.provider.clone(),
            model: row.model.clone(),
            agent: row.agent.clone(),
            scalar_features: row.scalar_features.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceTransition {
    pub context: TraceWindow,
    pub action: TraceAction,
    pub target: TraceWindow,
    pub labels: WorldLabelSet,
}

pub trait WorldRepresentationAdapter: Send + Sync {
    fn dimensions(&self) -> usize;
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn encode_state(&self, window: &TraceWindow) -> Result<Vec<f32>>;
    fn encode_action(&self, action: &TraceAction) -> Result<Vec<f32>>;
    fn encode_target(&self, window: &TraceWindow) -> Result<Vec<f32>>;

    /// Encode a batch of context windows into state vectors.
    ///
    /// Default: sequential loop over [`Self::encode_state`].  Override with a batched path
    /// (e.g. via [`WorldEmbeddingAdapter::embed_batch`]) to reduce embedding round-trips.
    fn encode_state_batch(&self, windows: &[TraceWindow]) -> Result<Vec<Vec<f32>>> {
        windows.iter().map(|w| self.encode_state(w)).collect()
    }

    /// Encode a batch of actions into action vectors.
    ///
    /// Default: sequential loop over [`Self::encode_action`].
    fn encode_action_batch(&self, actions: &[TraceAction]) -> Result<Vec<Vec<f32>>> {
        actions.iter().map(|a| self.encode_action(a)).collect()
    }

    /// Encode a batch of target windows into target vectors.
    ///
    /// Default: sequential loop over [`Self::encode_target`].
    fn encode_target_batch(&self, windows: &[TraceWindow]) -> Result<Vec<Vec<f32>>> {
        windows.iter().map(|w| self.encode_target(w)).collect()
    }
}

pub struct GenericEmbeddingRepresentationAdapter {
    inner: Box<dyn WorldEmbeddingAdapter>,
    redaction_policy: String,
}

impl GenericEmbeddingRepresentationAdapter {
    pub fn new(inner: Box<dyn WorldEmbeddingAdapter>) -> Self {
        Self {
            inner,
            redaction_policy: "world-model-default-redacted".into(),
        }
    }

    pub fn with_redaction_policy(mut self, redaction_policy: impl Into<String>) -> Self {
        self.redaction_policy = redaction_policy.into();
        self
    }

    fn embed(&self, source_hash: String, text: String) -> Result<Vec<f32>> {
        Ok(self
            .inner
            .embed(&EmbeddingRequest {
                text,
                source_hash,
                redaction_policy: self.redaction_policy.clone(),
            })?
            .values)
    }
}

impl WorldRepresentationAdapter for GenericEmbeddingRepresentationAdapter {
    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn encode_state(&self, window: &TraceWindow) -> Result<Vec<f32>> {
        self.embed(
            window_source_hash(window, "state"),
            window_embedding_text(window, "state"),
        )
    }

    fn encode_action(&self, action: &TraceAction) -> Result<Vec<f32>> {
        self.embed(
            format!("action:{}", action.action_ref),
            action_embedding_text(action),
        )
    }

    fn encode_target(&self, window: &TraceWindow) -> Result<Vec<f32>> {
        self.embed(
            window_source_hash(window, "target"),
            window_embedding_text(window, "target"),
        )
    }

    fn encode_state_batch(&self, windows: &[TraceWindow]) -> Result<Vec<Vec<f32>>> {
        let requests: Vec<EmbeddingRequest> = windows
            .iter()
            .map(|w| EmbeddingRequest {
                text: window_embedding_text(w, "state"),
                source_hash: window_source_hash(w, "state"),
                redaction_policy: self.redaction_policy.clone(),
            })
            .collect();
        self.inner
            .embed_batch(&requests)
            .map(|vs| vs.into_iter().map(|v| v.values).collect())
    }

    fn encode_action_batch(&self, actions: &[TraceAction]) -> Result<Vec<Vec<f32>>> {
        let requests: Vec<EmbeddingRequest> = actions
            .iter()
            .map(|a| EmbeddingRequest {
                text: action_embedding_text(a),
                source_hash: format!("action:{}", a.action_ref),
                redaction_policy: self.redaction_policy.clone(),
            })
            .collect();
        self.inner
            .embed_batch(&requests)
            .map(|vs| vs.into_iter().map(|v| v.values).collect())
    }

    fn encode_target_batch(&self, windows: &[TraceWindow]) -> Result<Vec<Vec<f32>>> {
        let requests: Vec<EmbeddingRequest> = windows
            .iter()
            .map(|w| EmbeddingRequest {
                text: window_embedding_text(w, "target"),
                source_hash: window_source_hash(w, "target"),
                redaction_policy: self.redaction_policy.clone(),
            })
            .collect();
        self.inner
            .embed_batch(&requests)
            .map(|vs| vs.into_iter().map(|v| v.values).collect())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraceWindowBuilder {
    rows: Vec<WorldTraceRow>,
}

impl TraceWindowBuilder {
    pub fn new(rows: &[WorldTraceRow]) -> Self {
        let mut rows = rows.to_vec();
        rows.sort_by(|left, right| {
            left.session_id
                .cmp(&right.session_id)
                .then_with(|| left.created_at.cmp(&right.created_at))
                .then_with(|| left.row_id.cmp(&right.row_id))
        });
        Self { rows }
    }

    pub fn context_window(&self, anchor_row_id: &str, context_rows: usize) -> Result<TraceWindow> {
        let index = self.index_of(anchor_row_id)?;
        self.context_window_at(index, context_rows)
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
                context: self.context_window_at(index, context_rows)?,
                action: TraceAction::from_row(current),
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

    fn window_from_range(
        &self,
        anchor_index: usize,
        start: usize,
        end: usize,
        horizon: usize,
    ) -> Result<TraceWindow> {
        if start >= end || end > self.rows.len() {
            bail!("invalid trace window range");
        }
        let anchor = &self.rows[anchor_index];
        Ok(TraceWindow {
            session_id: anchor.session_id.clone(),
            anchor_row_id: anchor.row_id.clone(),
            rows: self.rows[start..end].to_vec(),
            horizon,
            graph_context: graph_context_for_row(&self.rows, anchor),
        })
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

fn window_source_hash(window: &TraceWindow, role: &str) -> String {
    let row_ids = window
        .rows
        .iter()
        .map(|row| row.row_id.as_str())
        .collect::<Vec<_>>()
        .join("|");
    format!(
        "{role}:{}:h{}:{}",
        window.anchor_row_id, window.horizon, row_ids
    )
}

fn window_embedding_text(window: &TraceWindow, role: &str) -> String {
    let rows = window
        .rows
        .iter()
        .map(row_text)
        .collect::<Vec<_>>()
        .join(" | ");
    format!(
        "{role} session={} anchor={} horizon={} {} rows={}",
        window.session_id,
        window.anchor_row_id,
        window.horizon,
        window.graph_context.compact_text(),
        rows
    )
}

fn action_embedding_text(action: &TraceAction) -> String {
    format!(
        "action ref={} kind={:?} provider={} model={} agent={} cost={} duration={} attempt={} tokens_in={} tokens_out={} text={}",
        action.action_ref,
        action.action_kind,
        action.provider.as_deref().unwrap_or(""),
        action.model.as_deref().unwrap_or(""),
        action.agent.as_deref().unwrap_or(""),
        action
            .scalar_features
            .cost_usd
            .map(|value| value.to_string())
            .unwrap_or_default(),
        action
            .scalar_features
            .duration_ms
            .map(|value| value.to_string())
            .unwrap_or_default(),
        action
            .scalar_features
            .attempt_index
            .map(|value| value.to_string())
            .unwrap_or_default(),
        action
            .scalar_features
            .tokens_in
            .map(|value| value.to_string())
            .unwrap_or_default(),
        action
            .scalar_features
            .tokens_out
            .map(|value| value.to_string())
            .unwrap_or_default(),
        action.summary
    )
}

fn row_text(row: &WorldTraceRow) -> String {
    format!(
        "row={} source={:?} action={:?} provider={} model={} agent={} text={}",
        row.row_id,
        row.source,
        row.action_kind,
        row.provider.as_deref().unwrap_or(""),
        row.model.as_deref().unwrap_or(""),
        row.agent.as_deref().unwrap_or(""),
        row.redacted_excerpt.as_deref().unwrap_or("")
    )
}
