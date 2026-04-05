use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Block classification for cache optimization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockType {
    /// Cacheable content: personality, instructions, CLAUDE.md, tool definitions.
    Static,
    /// Changes per turn: memory, rules, inner_voice, dynamic, environment.
    Dynamic,
}

/// A prompt block with cache metadata.
#[derive(Debug, Clone)]
pub struct CacheBlock {
    pub content: String,
    pub block_type: BlockType,
    pub cache_control: Option<serde_json::Value>,
    /// Hash of `content` for change detection across turns.
    pub content_hash: u64,
}

/// Input sections for cache classification. Mirrors the fields of
/// `AssemblyInput` from `archon-consciousness` without creating a dependency.
#[derive(Debug, Clone, Default)]
pub struct SectionInput {
    pub identity: Option<String>,
    pub personality: Option<String>,
    pub rules: Option<String>,
    pub memories: Option<String>,
    pub user_prompt: Option<String>,
    pub project_instructions: Option<String>,
    pub environment: Option<String>,
    pub inner_voice: Option<String>,
    pub dynamic: Option<String>,
}

/// Cache hit-rate tracker.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub total_input_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub requests: u32,
}

impl CacheStats {
    /// Record a new API response's cache metrics.
    pub fn update(&mut self, creation: u64, read: u64, total_input: u64) {
        self.cache_creation_tokens += creation;
        self.cache_read_tokens += read;
        self.total_input_tokens += total_input;
        self.requests += 1;
    }

    /// Cache hit rate as a percentage (0.0 – 100.0).
    /// Returns 0.0 when no input tokens have been recorded.
    pub fn hit_rate(&self) -> f64 {
        if self.total_input_tokens == 0 {
            return 0.0;
        }
        self.cache_read_tokens as f64 / self.total_input_tokens as f64 * 100.0
    }

    /// Estimated token savings from cache reads.
    /// Cache reads cost 90 % less than regular input, so each cached token
    /// saves 0.9 tokens worth of cost.
    pub fn estimated_savings(&self) -> f64 {
        self.cache_read_tokens as f64 * 0.9
    }

    /// Formatted string suitable for the `/cost` slash command.
    pub fn format_for_cost(&self) -> String {
        format!(
            "Cache hit rate: {:.1}% ({} reads / {} total)\n\
             Cache creation: {} tokens\n\
             Estimated savings: {:.0} token-equivalents",
            self.hit_rate(),
            self.cache_read_tokens,
            self.total_input_tokens,
            self.cache_creation_tokens,
            self.estimated_savings(),
        )
    }
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// Classify sections from a [`SectionInput`] into static and dynamic cache blocks.
///
/// Static sections (cacheable across turns):
///   identity, personality, project_instructions, user_prompt
///
/// Dynamic sections (change per turn):
///   rules, memories, inner_voice, dynamic, environment
pub fn classify_blocks(input: &SectionInput) -> Vec<CacheBlock> {
    let mut blocks = Vec::new();

    let static_fields: &[(&Option<String>,)] = &[
        (&input.identity,),
        (&input.personality,),
        (&input.project_instructions,),
        (&input.user_prompt,),
    ];

    for (field,) in static_fields {
        if let Some(text) = field
            && !text.is_empty()
        {
            blocks.push(CacheBlock {
                content_hash: hash_content(text),
                content: text.clone(),
                block_type: BlockType::Static,
                cache_control: None,
            });
        }
    }

    let dynamic_fields: &[(&Option<String>,)] = &[
        (&input.rules,),
        (&input.memories,),
        (&input.inner_voice,),
        (&input.dynamic,),
        (&input.environment,),
    ];

    for (field,) in dynamic_fields {
        if let Some(text) = field
            && !text.is_empty()
        {
            blocks.push(CacheBlock {
                content_hash: hash_content(text),
                content: text.clone(),
                block_type: BlockType::Dynamic,
                cache_control: None,
            });
        }
    }

    blocks
}

// ---------------------------------------------------------------------------
// Optimization
// ---------------------------------------------------------------------------

/// Reorder blocks so statics come first and dynamics last.
/// When `cache_enabled` is true, the last static block receives a
/// `cache_control: { "type": "ephemeral" }` hint so the API caches
/// everything up to (and including) that block.
pub fn optimize_block_order(blocks: Vec<CacheBlock>, cache_enabled: bool) -> Vec<CacheBlock> {
    let mut statics: Vec<CacheBlock> = blocks
        .iter()
        .filter(|b| b.block_type == BlockType::Static)
        .cloned()
        .collect();
    let dynamics: Vec<CacheBlock> = blocks
        .iter()
        .filter(|b| b.block_type == BlockType::Dynamic)
        .cloned()
        .collect();

    // Apply cache_control to the last static block when caching is on.
    if cache_enabled && let Some(last_static) = statics.last_mut() {
        last_static.cache_control = Some(serde_json::json!({"type": "ephemeral"}));
    }

    statics.extend(dynamics);
    statics
}

// ---------------------------------------------------------------------------
// API conversion
// ---------------------------------------------------------------------------

/// Convert cache blocks into JSON content blocks suitable for the Anthropic
/// messages API `system` parameter (array-of-objects format).
pub fn to_api_blocks(blocks: &[CacheBlock]) -> Vec<serde_json::Value> {
    blocks
        .iter()
        .map(|block| {
            let mut obj = serde_json::json!({
                "type": "text",
                "text": block.content,
            });
            if let Some(ref cc) = block.cache_control {
                obj["cache_control"] = cc.clone();
            }
            obj
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute a quick content hash using `DefaultHasher`.
fn hash_content(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}
