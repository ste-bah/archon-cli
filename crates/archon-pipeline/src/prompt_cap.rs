//! Token-aware prompt truncation.
//!
//! Truncates assembled prompt layers to fit within 80% of the model context
//! window, removing lowest-priority layers first and partially truncating
//! when full removal would overshoot.

use anyhow::Result;

/// Priority levels for truncation — lower numeric value = removed first.
///
/// Truncation order per REQ-PIPE-006:
/// 1. L3 (LEANN semantic context) — truncated first
/// 2. L5-L9 (learning layers) — truncated second
/// 3. L4 (RLM namespace context) — truncated third
/// 4. L2 (task context) — truncated fourth (via Required fallback)
/// 5. L1 (base prompt) — NEVER truncated
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TruncationPriority {
    /// Priority 1 — removed first (L3 LEANN semantic context).
    LeannSemanticContext,
    /// Priority 2 (L5 DESC episodes).
    DescEpisodes,
    /// Priority 3 (L6 SONA patterns).
    SonaPatterns,
    /// Priority 4 (L7 Reflexion trajectories).
    ReflexionTrajectories,
    /// Priority 5 (L8 PatternMatcher results).
    PatternMatcherResults,
    /// Priority 6 (L9 Sherlock verdicts).
    SherlockVerdicts,
    /// Priority 7 (L10 Algorithm strategy).
    AlgorithmStrategy,
    /// Priority 8 (L4 RLM namespace context).
    RlmContext,
    /// Priority 100 — never removed (L1 base prompt, L2 task context).
    Required,
}

impl TruncationPriority {
    fn ordinal(&self) -> u32 {
        match self {
            Self::LeannSemanticContext => 1,
            Self::DescEpisodes => 2,
            Self::SonaPatterns => 3,
            Self::ReflexionTrajectories => 4,
            Self::PatternMatcherResults => 5,
            Self::SherlockVerdicts => 6,
            Self::AlgorithmStrategy => 7,
            Self::RlmContext => 8,
            Self::Required => 100,
        }
    }
}

impl PartialOrd for TruncationPriority {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TruncationPriority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ordinal().cmp(&other.ordinal())
    }
}

/// A single layer of the assembled prompt.
#[derive(Clone, Debug)]
pub struct PromptLayer {
    pub name: String,
    pub content: String,
    pub priority: TruncationPriority,
    pub required: bool,
}

/// Result of truncating prompt layers to fit within the token budget.
pub struct TruncatedPrompt {
    /// Surviving layers (in original order).
    pub layers: Vec<PromptLayer>,
    /// Total token count of surviving layers.
    pub total_tokens: usize,
    /// Names of layers that were fully removed.
    pub removed_layers: Vec<String>,
    /// Layers that were partially truncated: (name, original_tokens, final_tokens).
    pub truncated_layers: Vec<(String, usize, usize)>,
}

/// Count tokens using a character-based heuristic: ceil(len / 4).
pub fn count_tokens(text: &str) -> usize {
    let len = text.len();
    if len == 0 {
        return 0;
    }
    (len + 3) / 4
}

/// Truncate content to approximately `target_tokens` tokens by keeping the
/// first `target_tokens * 4` characters.
fn truncate_content(content: &str, target_tokens: usize) -> String {
    let max_chars = target_tokens * 4;
    if content.len() <= max_chars {
        content.to_string()
    } else {
        content[..max_chars].to_string()
    }
}

/// Truncate layers to fit within 80% of `model_context_window`.
///
/// Algorithm:
/// 1. Calculate target = model_context_window * 4 / 5
/// 2. If total tokens fit, return unchanged.
/// 3. Sort non-required layers by priority ascending (lowest removed first).
/// 4. Remove/truncate layers until budget is met.
/// 5. If still over budget after removing all non-required layers, truncate
///    the `task_context` required layer (or last required layer).
pub fn truncate_prompt(
    layers: Vec<PromptLayer>,
    model_context_window: usize,
) -> Result<TruncatedPrompt> {
    let target = model_context_window * 4 / 5;

    // Calculate initial total.
    let total: usize = layers.iter().map(|l| count_tokens(&l.content)).sum();

    if total <= target {
        return Ok(TruncatedPrompt {
            layers,
            total_tokens: total,
            removed_layers: Vec::new(),
            truncated_layers: Vec::new(),
        });
    }

    // Separate required and non-required layers, preserving original indices.
    let mut removable: Vec<(usize, &PromptLayer)> = layers
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.required)
        .collect();

    // Sort removable by priority ascending (lowest ordinal first = removed first).
    removable.sort_by(|a, b| a.1.priority.cmp(&b.1.priority));

    let mut removed_layers: Vec<String> = Vec::new();
    let mut truncated_layers: Vec<(String, usize, usize)> = Vec::new();
    // Track which indices are removed or have modified content.
    let mut removed_indices: Vec<usize> = Vec::new();
    // Map from index to new content for partially truncated layers.
    let mut modified_content: std::collections::HashMap<usize, String> =
        std::collections::HashMap::new();

    let mut current_total = total;

    let removable_count = removable.len();
    for (pos, (idx, layer)) in removable.iter().enumerate() {
        if current_total <= target {
            break;
        }
        let layer_tokens = count_tokens(&layer.content);
        if layer_tokens == 0 {
            continue;
        }
        let excess = current_total - target;
        let is_last_removable = pos == removable_count - 1;

        // Partially truncate only if this is the last removable layer and
        // its token count exceeds the remaining excess (i.e., we can keep
        // some of its content and still fit within budget).
        if is_last_removable && layer_tokens > excess {
            let keep_tokens = layer_tokens - excess;
            let new_content = truncate_content(&layer.content, keep_tokens);
            let new_tokens = count_tokens(&new_content);
            truncated_layers.push((layer.name.clone(), layer_tokens, new_tokens));
            modified_content.insert(*idx, new_content);
            current_total = current_total - layer_tokens + new_tokens;
        } else {
            // Fully remove this layer.
            removed_layers.push(layer.name.clone());
            removed_indices.push(*idx);
            current_total -= layer_tokens;
        }
    }

    // If still over budget, truncate required layers.
    if current_total > target {
        // Find the task_context layer (required, name contains "task").
        let task_idx = layers
            .iter()
            .enumerate()
            .find(|(_, l)| l.required && l.name.contains("task"))
            .map(|(i, _)| i);

        let truncate_idx = task_idx.unwrap_or_else(|| {
            // Fall back to the last required layer.
            layers
                .iter()
                .enumerate()
                .filter(|(_, l)| l.required)
                .map(|(i, _)| i)
                .last()
                .expect("at least one required layer must exist")
        });

        let layer = &layers[truncate_idx];
        let layer_tokens = count_tokens(
            modified_content
                .get(&truncate_idx)
                .map(|s| s.as_str())
                .unwrap_or(&layer.content),
        );
        let excess = current_total - target;
        let keep_tokens = if layer_tokens > excess {
            layer_tokens - excess
        } else {
            1 // Keep at least something.
        };
        let source = modified_content
            .get(&truncate_idx)
            .cloned()
            .unwrap_or_else(|| layer.content.clone());
        let new_content = truncate_content(&source, keep_tokens);
        let new_tokens = count_tokens(&new_content);
        truncated_layers.push((layer.name.clone(), count_tokens(&layer.content), new_tokens));
        modified_content.insert(truncate_idx, new_content);
        // current_total updated for consistency; not read after this block.
        let _ = current_total - layer_tokens + new_tokens;
    }

    // Build the surviving layers list, preserving original order.
    let mut surviving_layers: Vec<PromptLayer> = Vec::new();
    for (idx, layer) in layers.into_iter().enumerate() {
        if removed_indices.contains(&idx) {
            continue;
        }
        if let Some(new_content) = modified_content.remove(&idx) {
            surviving_layers.push(PromptLayer {
                name: layer.name,
                content: new_content,
                priority: layer.priority,
                required: layer.required,
            });
        } else {
            surviving_layers.push(layer);
        }
    }

    let final_total: usize = surviving_layers
        .iter()
        .map(|l| count_tokens(&l.content))
        .sum();

    Ok(TruncatedPrompt {
        layers: surviving_layers,
        total_tokens: final_total,
        removed_layers,
        truncated_layers,
    })
}
