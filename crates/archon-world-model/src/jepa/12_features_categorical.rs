// Reserved dimensions at the tail of the feature vector for the categoricals.
//
// `source` and `action_kind` are closed enums, so every value owns a dimension
// outright and cannot collide with anything. `provider`, `agent` and `model`
// are open strings — there is no canonical provider enum in the workspace and a
// new provider or agent name can appear at any time — so each gets its own
// small sub-block instead. Collisions are then confined to a handful of values
// within one field rather than landing on top of an open English vocabulary,
// which is what happened when all five were hashed across the whole vector
// alongside every excerpt token.
const SOURCE_SLOTS: usize = 12;
// 13 since #184 M9 added the four coordination verbs. Widening this shifts
// PROVIDER_BASE and everything after it, so a checkpoint trained before the
// change encodes a different layout — `EVAL_SCHEMA_VERSION` and the projection
// version are bumped alongside so the mismatch is caught rather than silently
// compared.
const ACTION_KIND_SLOTS: usize = 13;
const PROVIDER_SLOTS: usize = 8;
const AGENT_SLOTS: usize = 8;
const MODEL_SLOTS: usize = 8;
const CATEGORICAL_SLOTS: usize =
    SOURCE_SLOTS + ACTION_KIND_SLOTS + PROVIDER_SLOTS + AGENT_SLOTS + MODEL_SLOTS;

const SOURCE_BASE: usize = 0;
const ACTION_KIND_BASE: usize = SOURCE_BASE + SOURCE_SLOTS;
const PROVIDER_BASE: usize = ACTION_KIND_BASE + ACTION_KIND_SLOTS;
const AGENT_BASE: usize = PROVIDER_BASE + PROVIDER_SLOTS;
const MODEL_BASE: usize = AGENT_BASE + AGENT_SLOTS;

/// Start of the reserved block, or `None` when the vector cannot spare it.
///
/// The block has to stay a small fraction of the vector or it crowds out the
/// embedding it sits beside. Small `latent_dim` values — including the ones the
/// tests use — fall below the threshold and keep the fully hashed behaviour.
fn categorical_base(dimensions: usize) -> Option<usize> {
    (dimensions >= CATEGORICAL_SLOTS * 4).then(|| dimensions - CATEGORICAL_SLOTS)
}

/// Slot for a closed-enum value.
///
/// Exhaustive on purpose: adding a variant must fail to compile here rather
/// than silently alias onto another value's dimension.
fn source_slot(source: &WorldTraceSource) -> usize {
    match source {
        WorldTraceSource::ActivityEvent => 0,
        WorldTraceSource::PipelineBundle => 1,
        WorldTraceSource::ProviderRuntime => 2,
        WorldTraceSource::Plan => 3,
        WorldTraceSource::Conversation => 4,
        WorldTraceSource::AgentTranscript => 5,
        WorldTraceSource::AgentOutput => 6,
        WorldTraceSource::Workflow => 7,
        WorldTraceSource::Retrospective => 8,
        WorldTraceSource::Memory => 9,
        WorldTraceSource::AgentEvolution => 10,
        WorldTraceSource::ReasoningQuality => 11,
    }
}

/// As [`source_slot`], and exhaustive for the same reason.
fn action_kind_slot(kind: &WorldActionKind) -> usize {
    match kind {
        WorldActionKind::AgentAttempt => 0,
        WorldActionKind::ProviderCall => 1,
        WorldActionKind::ToolCall => 2,
        WorldActionKind::PlanUpdate => 3,
        WorldActionKind::MemorySurface => 4,
        WorldActionKind::Verification => 5,
        WorldActionKind::Retry => 6,
        WorldActionKind::Resume => 7,
        WorldActionKind::MessageSend => 8,
        WorldActionKind::TaskClaim => 9,
        WorldActionKind::Handoff => 10,
        WorldActionKind::WorktreeMerge => 11,
        WorldActionKind::Unknown => 12,
    }
}

/// Sub-block index for an open-domain string.
fn open_slot(value: &str, slots: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    (hasher.finish() as usize) % slots
}

fn add_slot(features: &mut [f32], index: usize, weight: f32) {
    if weight.is_finite() && index < features.len() {
        features[index] += weight;
    }
}

/// The categorical values of a row or an action.
///
/// `TraceAction` carries no `source`, hence the `Option`.
struct Categoricals<'a> {
    source: Option<&'a WorldTraceSource>,
    action_kind: &'a WorldActionKind,
    provider: Option<&'a String>,
    model: Option<&'a String>,
    agent: Option<&'a String>,
}

/// Relative importance of each categorical. Rows and actions weight them
/// differently — an action's own kind matters more than the kind of a row
/// somewhere in its context window.
struct CategoricalWeights {
    source: f32,
    action_kind: f32,
    provider: f32,
    model: f32,
    agent: f32,
}

/// Write the categoricals, into reserved slots where the vector allows it.
///
/// The `role` prefix is dropped on the reserved path. It never carried
/// information — context, target and action vectors are built separately and
/// fed to separate encoders, never concatenated — so all it did was send the
/// same value to a different bucket per role. A stable slot is more useful.
fn add_categorical_features(
    features: &mut [f32],
    values: Categoricals<'_>,
    weights: CategoricalWeights,
    weight: f32,
    role: &str,
) {
    match categorical_base(features.len()) {
        Some(base) => {
            if let Some(source) = values.source {
                add_slot(
                    features,
                    base + SOURCE_BASE + source_slot(source),
                    weights.source * weight,
                );
            }
            add_slot(
                features,
                base + ACTION_KIND_BASE + action_kind_slot(values.action_kind),
                weights.action_kind * weight,
            );
            if let Some(provider) = values.provider {
                add_slot(
                    features,
                    base + PROVIDER_BASE + open_slot(provider, PROVIDER_SLOTS),
                    weights.provider * weight,
                );
            }
            if let Some(model) = values.model {
                add_slot(
                    features,
                    base + MODEL_BASE + open_slot(model, MODEL_SLOTS),
                    weights.model * weight,
                );
            }
            if let Some(agent) = values.agent {
                add_slot(
                    features,
                    base + AGENT_BASE + open_slot(agent, AGENT_SLOTS),
                    weights.agent * weight,
                );
            }
        }
        None => {
            if let Some(source) = values.source {
                add_token(
                    features,
                    &format!("{role}:source:{source:?}"),
                    weights.source * weight,
                );
            }
            add_token(
                features,
                &format!("{role}:action_kind:{:?}", values.action_kind),
                weights.action_kind * weight,
            );
            if let Some(provider) = values.provider {
                add_token(
                    features,
                    &format!("{role}:provider:{provider}"),
                    weights.provider * weight,
                );
            }
            if let Some(model) = values.model {
                add_token(
                    features,
                    &format!("{role}:model:{model}"),
                    weights.model * weight,
                );
            }
            if let Some(agent) = values.agent {
                add_token(
                    features,
                    &format!("{role}:agent:{agent}"),
                    weights.agent * weight,
                );
            }
        }
    }
}

