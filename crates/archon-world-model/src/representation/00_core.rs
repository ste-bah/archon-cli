#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceWindow {
    pub session_id: String,
    pub anchor_row_id: String,
    pub rows: Vec<WorldTraceRow>,
    pub horizon: usize,
    pub graph_context: GraphContextFeatures,
    /// Dense semantic vector for this window, sized to the model's latent
    /// dimension, or `None` when no embedding was available.
    ///
    /// Populated by the caller, never inside `jepa/`: the encoder path is
    /// gated to stay free of embedding adapters, and this keeps the dependency
    /// pointing the right way — the caller chooses a provider (including
    /// `deterministic-hash`, which needs nothing external) and hands the
    /// encoder plain numbers.
    ///
    /// Without it the encoder falls back to hashing every excerpt token into
    /// `latent_dim` buckets, which at the default 384 collides an open
    /// vocabulary into ~384 slots and hands the model a sum of unrelated
    /// features.
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
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
    /// Dense semantic vector for `summary`. See [`TraceWindow::embedding`].
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
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
            embedding: None,
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

/// Embed `text`, discarding any failure.
///
/// Every caller wants "a vector if one is available", never an error: the
/// encoder has a working fallback, so a missing model or an offline provider
/// should cost representation quality, not the whole run.
fn embed_text(embedder: &dyn WorldEmbeddingAdapter, source_hash: String, text: &str) -> Option<Vec<f32>> {
    let request = EmbeddingRequest {
        text: text.to_string(),
        source_hash,
        // The excerpt was already redacted when the row was ingested, so this
        // records that rather than requesting further redaction.
        redaction_policy: "row-redacted".to_string(),
    };
    embedder
        .embed(&request)
        .ok()
        .map(|vector| vector.values)
        .filter(|values| !values.is_empty() && values.iter().all(|value| value.is_finite()))
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
