fn window_features(window: &TraceWindow, dimensions: usize, role: &str) -> Result<Vec<f32>> {
    if dimensions == 0 {
        bail!("jepa dimensions must be greater than zero");
    }
    let mut features = vec![0.0; dimensions];
    add_token(
        &mut features,
        &format!("{role}:session:{}", window.session_id),
        0.10,
    );
    add_token(
        &mut features,
        &format!("{role}:anchor:{}", window.anchor_row_id),
        0.05,
    );
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
    for plan_id in &window.graph_context.prior_plan_ids {
        add_token(&mut features, &format!("graph.plan:{plan_id}"), 0.10);
    }
    for memory_id in &window.graph_context.prior_memory_ids {
        add_token(&mut features, &format!("graph.memory:{memory_id}"), 0.10);
    }

    let row_weight = 1.0 / window.rows.len().max(1) as f32;
    for row in &window.rows {
        add_row_features(&mut features, row, row_weight, role);
    }
    normalize(&mut features);
    Ok(features)
}

fn action_features(action: &TraceAction, dimensions: usize, role: &str) -> Result<Vec<f32>> {
    if dimensions == 0 {
        bail!("jepa dimensions must be greater than zero");
    }
    let mut features = vec![0.0; dimensions];
    add_token(
        &mut features,
        &format!("{role}:action:{}", action.action_ref),
        0.20,
    );
    add_token(
        &mut features,
        &format!("{role}:kind:{:?}", action.action_kind),
        0.80,
    );
    if let Some(provider) = &action.provider {
        add_token(&mut features, &format!("{role}:provider:{provider}"), 0.65);
    }
    if let Some(model) = &action.model {
        add_token(&mut features, &format!("{role}:model:{model}"), 0.50);
    }
    if let Some(agent) = &action.agent {
        add_token(&mut features, &format!("{role}:agent:{agent}"), 0.50);
    }
    add_scalar_features(&mut features, &action.scalar_features, 1.0);
    add_lexical_features(&mut features, &action.summary, 0.20);
    normalize(&mut features);
    Ok(features)
}

fn add_row_features(features: &mut [f32], row: &WorldTraceRow, weight: f32, role: &str) {
    add_token(
        features,
        &format!("{role}:source:{:?}", row.source),
        0.45 * weight,
    );
    add_token(
        features,
        &format!("{role}:action_kind:{:?}", row.action_kind),
        0.65 * weight,
    );
    if let Some(provider) = &row.provider {
        add_token(
            features,
            &format!("{role}:provider:{provider}"),
            0.55 * weight,
        );
    }
    if let Some(model) = &row.model {
        add_token(features, &format!("{role}:model:{model}"), 0.40 * weight);
    }
    if let Some(agent) = &row.agent {
        add_token(features, &format!("{role}:agent:{agent}"), 0.40 * weight);
    }
    add_scalar_features(features, &row.scalar_features, weight);
    if let Some(excerpt) = &row.redacted_excerpt {
        add_lexical_features(features, excerpt, 0.15 * weight);
    }
    for evidence in &row.evidence_refs {
        add_token(
            features,
            &format!("{role}:evidence:{}:{}", evidence.source, evidence.id),
            0.10 * weight,
        );
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
