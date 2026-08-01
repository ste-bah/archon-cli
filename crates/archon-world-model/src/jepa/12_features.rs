fn window_features(window: &TraceWindow, dimensions: usize, role: &str) -> Result<Vec<f32>> {
    if dimensions == 0 {
        bail!("jepa dimensions must be greater than zero");
    }
    // Start from the dense semantic vector when the caller supplied one. Every
    // dimension then means something on its own, and the structured features
    // below are added on top rather than colliding with an open vocabulary.
    let mut features = seed_features(window.embedding.as_deref(), dimensions);
    // `session_id` and `anchor_row_id` are deliberately NOT hashed in. Both are
    // unbounded identifiers whose values appear once and never recur, so they
    // cannot generalise to a new row — they only consume bucket capacity and
    // add noise proportional to the corpus size.
    add_numeric(
        &mut features,
        "horizon",
        normalized_horizon(window.horizon),
        0.50,
    );
    add_numeric(
        &mut features,
        "graph.session_neighbor_count",
        normalize_count(window.graph_context.session_neighbor_count),
        0.55,
    );
    add_numeric(
        &mut features,
        "graph.same_agent_prior_count",
        normalize_count(window.graph_context.same_agent_prior_count),
        0.45,
    );
    add_numeric(
        &mut features,
        "graph.same_provider_prior_count",
        normalize_count(window.graph_context.same_provider_prior_count),
        0.45,
    );
    add_numeric(
        &mut features,
        "graph.prior_plan_updates",
        normalize_count(window.graph_context.prior_plan_updates),
        0.40,
    );
    add_numeric(
        &mut features,
        "graph.prior_memory_surfaces",
        normalize_count(window.graph_context.prior_memory_surfaces),
        0.40,
    );
    // Prior plan/memory *counts* are kept above; their *ids* are not. Same
    // reason as session/anchor: one occurrence each, no generalisation.

    // Hashing the excerpts too would scatter an open vocabulary across the same
    // dimensions the embedding just filled, undoing it. Lexical hashing is the
    // fallback for when there is no embedding, not a supplement to one.
    let lexical = LexicalFallback::for_embedding(window.embedding.as_deref(), dimensions);
    let row_weight = 1.0 / window.rows.len().max(1) as f32;
    for row in &window.rows {
        add_row_features(&mut features, row, row_weight, role, lexical);
    }
    normalize(&mut features);
    Ok(features)
}

fn action_features(action: &TraceAction, dimensions: usize, role: &str) -> Result<Vec<f32>> {
    if dimensions == 0 {
        bail!("jepa dimensions must be greater than zero");
    }
    let mut features = seed_features(action.embedding.as_deref(), dimensions);
    // `action_ref` is a row id — unbounded and single-occurrence, so it cannot
    // generalise. Dropped for the same reason as the window identifiers.
    add_categorical_features(
        &mut features,
        Categoricals {
            source: None,
            action_kind: &action.action_kind,
            provider: action.provider.as_ref(),
            model: action.model.as_ref(),
            agent: action.agent.as_ref(),
        },
        CategoricalWeights {
            source: 0.0,
            action_kind: 0.80,
            provider: 0.65,
            model: 0.50,
            agent: 0.50,
        },
        1.0,
        role,
    );
    add_scalar_features(&mut features, &action.scalar_features, 1.0);
    if LexicalFallback::for_embedding(action.embedding.as_deref(), dimensions).is_enabled() {
        add_lexical_features(&mut features, &action.summary, 0.20);
    }
    normalize(&mut features);
    Ok(features)
}

/// Whether excerpt text still needs hashing into the feature buckets.
///
/// Disabled once a dense embedding is present: the embedding already carries
/// the text, and hashing it again would scatter an open vocabulary over the
/// dimensions the embedding just set.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LexicalFallback {
    Enabled,
    Disabled,
}

impl LexicalFallback {
    fn for_embedding(embedding: Option<&[f32]>, dimensions: usize) -> Self {
        match usable_embedding(embedding, dimensions) {
            Some(_) => Self::Disabled,
            None => Self::Enabled,
        }
    }

    fn is_enabled(self) -> bool {
        self == Self::Enabled
    }
}

fn add_row_features(
    features: &mut [f32],
    row: &WorldTraceRow,
    weight: f32,
    role: &str,
    lexical: LexicalFallback,
) {
    add_categorical_features(
        features,
        Categoricals {
            source: Some(&row.source),
            action_kind: &row.action_kind,
            provider: row.provider.as_ref(),
            model: row.model.as_ref(),
            agent: row.agent.as_ref(),
        },
        CategoricalWeights {
            source: 0.45,
            action_kind: 0.65,
            provider: 0.55,
            model: 0.40,
            agent: 0.40,
        },
        weight,
        role,
    );
    add_scalar_features(features, &row.scalar_features, weight);
    if let Some(excerpt) = row.redacted_excerpt.as_ref().filter(|_| lexical.is_enabled()) {
        add_lexical_features(features, excerpt, 0.15 * weight);
    }
    // Evidence *ids* are not hashed in: unbounded and single-occurrence, so no
    // generalisation. The evidence *source* is low-cardinality and does carry
    // signal, so that stays.
    for evidence in &row.evidence_refs {
        add_token(
            features,
            &format!("{role}:evidence_source:{}", evidence.source),
            0.10 * weight,
        );
    }
}

/// The embedding to build features from, or `None` when it cannot be used.
///
/// A vector of the wrong length, or one carrying a non-finite value, is not
/// usable. Both the seed and the lexical-fallback decision go through here so
/// that they cannot disagree. They used to apply different rules: an embedding
/// of the wrong length zeroed the seed *and* suppressed lexical hashing, so the
/// excerpt text contributed nothing at all and the result was worse than either
/// path alone. Providers do return unexpected dimensions, so this is reachable
/// rather than theoretical.
///
/// A wrong-width embedding is ignored rather than truncated or padded: it means
/// the corpus was embedded under a different projection than the model expects,
/// and silently reshaping it would train on two incompatible representations.
fn usable_embedding(embedding: Option<&[f32]>, dimensions: usize) -> Option<&[f32]> {
    embedding.filter(|values| {
        values.len() == dimensions && values.iter().all(|value| value.is_finite())
    })
}

/// Base feature vector for an encoder input.
///
/// With a caller-supplied embedding of the right width, that vector *is* the
/// base and the structured features are added on top. Otherwise the base is
/// zeros and everything falls back to hashing, which is the pre-embedding
/// behaviour.
fn seed_features(embedding: Option<&[f32]>, dimensions: usize) -> Vec<f32> {
    match usable_embedding(embedding, dimensions) {
        Some(values) => values.to_vec(),
        None => vec![0.0; dimensions],
    }
}

fn add_scalar_features(features: &mut [f32], scalar: &ScalarFeatures, weight: f32) {
    if let Some(value) = scalar.cost_usd {
        add_numeric(
            features,
            "scalar.cost_usd",
            (value as f32 / 2.0).clamp(0.0, 8.0),
            weight,
        );
    }
    if let Some(value) = scalar.duration_ms {
        add_numeric(
            features,
            "scalar.duration_ms",
            (value as f32 / 300_000.0).clamp(0.0, 8.0),
            weight,
        );
    }
    if let Some(value) = scalar.attempt_index {
        add_numeric(
            features,
            "scalar.attempt_index",
            (value as f32 / 8.0).clamp(0.0, 4.0),
            weight,
        );
    }
    if let Some(value) = scalar.tokens_in {
        add_numeric(
            features,
            "scalar.tokens_in",
            (value as f32 / 100_000.0).clamp(0.0, 8.0),
            weight,
        );
    }
    if let Some(value) = scalar.tokens_out {
        add_numeric(
            features,
            "scalar.tokens_out",
            (value as f32 / 50_000.0).clamp(0.0, 8.0),
            weight,
        );
    }
    if let Some(value) = scalar.quality_overall {
        add_numeric(
            features,
            "scalar.quality_overall",
            (value as f32).clamp(0.0, 1.0),
            weight,
        );
    }
    if let Some(value) = scalar.provider_cooldown_ms {
        add_numeric(
            features,
            "scalar.provider_cooldown_ms",
            (value as f32 / 300_000.0).clamp(0.0, 8.0),
            weight,
        );
    }
}

fn add_lexical_features(features: &mut [f32], text: &str, weight: f32) {
    for token in text.split_whitespace().take(64) {
        add_token(features, &format!("lex:{token}"), weight);
    }
}

fn add_numeric(features: &mut [f32], name: &str, value: f32, weight: f32) {
    if value.is_finite() {
        add_token(features, &format!("num:{name}"), value * weight);
    }
}

fn add_token(features: &mut [f32], token: &str, weight: f32) {
    if features.is_empty() || !weight.is_finite() {
        return;
    }
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    let hash = hasher.finish();
    let bucket = (hash as usize) % features.len();
    let sign = if hash & 1 == 0 { 1.0 } else { -1.0 };
    features[bucket] += sign * weight;
}

fn deterministic_vector(
    role: &str,
    salt: &str,
    len: usize,
    min_value: f32,
    max_value: f32,
) -> Vec<f32> {
    (0..len)
        .map(|idx| {
            let mut hasher = DefaultHasher::new();
            role.hash(&mut hasher);
            salt.hash(&mut hasher);
            idx.hash(&mut hasher);
            let unit = (hasher.finish() % 10_000) as f32 / 10_000.0;
            min_value + unit * (max_value - min_value)
        })
        .collect()
}

fn ema_values(previous_target: &[f32], online: &[f32], decay: f32) -> Vec<f32> {
    previous_target
        .iter()
        .zip(online)
        .map(|(target, online)| decay * target + (1.0 - decay) * online)
        .collect()
}
