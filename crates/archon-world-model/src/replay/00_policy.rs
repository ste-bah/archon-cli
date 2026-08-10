//! Replay policy, its hard bounds, and the surprise/held-out inputs.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::guardrail::WorldGuardrailOutcome;
use crate::schema::WorldTraceRow;

/// Largest share of a batch that may come from the prioritised stream.
///
/// The complement is the uniform floor: with `f <= 0.5` every training
/// transition keeps a selection probability of at least `0.5/n`, which is what
/// bounds the importance weight by `1/(1 - f) = 2.0`. Raising this past 0.5
/// would let the correction term grow without limit, so it is a constant and
/// not a config key.
pub const MAX_PRIORITIZED_FRACTION: f32 = 0.5;

/// Largest weight ratio between the most and least surprising transition.
///
/// Four, applied to a *rank* percentile rather than the surprise value: a
/// single anomalous transition cannot buy more than 4x the attention of an
/// ordinary one however large its raw error was.
pub const MAX_SURPRISE_WEIGHT: f32 = 4.0;

/// Largest share of a batch any one priority decile may supply.
///
/// The roadmap's W6 automatic-rollback trigger fires when one priority decile
/// supplies over 40% of a batch. Enforced during the draw so the monitor
/// observes an invariant rather than a hope.
pub const MAX_DECILE_SHARE: f32 = 0.40;

/// Share of sessions reserved for evaluation.
pub const DEFAULT_HELD_OUT_FRACTION: f32 = 0.2;

/// Fixed input to the held-out hash.
///
/// Changing it re-partitions every corpus, so it carries a version and lives
/// next to [`ReplayPolicy::split_version`] in the recorded summary.
pub const HELD_OUT_SPLIT_SALT: &str = "archon-world-model/replay/held-out/v1";

/// Identifies the priority scheme that produced a plan.
///
/// Recorded on every summary so two evaluation windows computed under different
/// schemes cannot be pooled by accident.
pub const PRIORITY_VERSION: &str = "w6-surprise-rank-v1";

/// Buckets the session hash is reduced to before the fraction is applied.
const SPLIT_BUCKETS: u64 = 10_000;

/// Default batch ceiling.
///
/// The W6 gate calls for matched 500-uniform / 500-prioritized evaluations;
/// 512 is the nearest power-of-two batch that covers one.
const DEFAULT_BATCH_SIZE: usize = 512;

/// Highest held-out share the split will honour.
///
/// A corpus that reserves more than half of itself has stopped being a training
/// corpus; the clamp makes a fat-fingered config a bounded mistake.
const MAX_HELD_OUT_FRACTION: f32 = 0.5;

/// How replay draws a batch, and what it is allowed to change.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ReplayPolicy {
    /// Whether the plan is *applied* to the trainer's example set.
    ///
    /// `false` by default. The plan is computed and reported either way; only
    /// this decides whether prioritisation changes what the model learns from.
    pub prioritized_enabled: bool,
    pub held_out_fraction: f32,
    pub batch_size: usize,
    pub prioritized_fraction: f32,
    pub max_surprise_weight: f32,
    pub max_decile_share: f32,
    /// Seed for the draw. Fixed so a run is reproducible from its summary.
    pub seed: u64,
    pub split_version: u32,
}

impl Default for ReplayPolicy {
    fn default() -> Self {
        Self {
            prioritized_enabled: false,
            held_out_fraction: DEFAULT_HELD_OUT_FRACTION,
            batch_size: DEFAULT_BATCH_SIZE,
            prioritized_fraction: MAX_PRIORITIZED_FRACTION,
            max_surprise_weight: MAX_SURPRISE_WEIGHT,
            max_decile_share: MAX_DECILE_SHARE,
            seed: 0x5713_2C9E_A1B4_0F17,
            split_version: 1,
        }
    }
}

impl ReplayPolicy {
    /// Operator values, clamped into the module's hard bounds.
    ///
    /// Config may only make replay more conservative. Every consumer calls this
    /// before reading a field, so a widened value in `config.toml` is narrowed
    /// here rather than trusted.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            prioritized_enabled: self.prioritized_enabled,
            held_out_fraction: clamp_finite(self.held_out_fraction, 0.0, MAX_HELD_OUT_FRACTION),
            batch_size: self.batch_size.max(1),
            prioritized_fraction: clamp_finite(
                self.prioritized_fraction,
                0.0,
                MAX_PRIORITIZED_FRACTION,
            ),
            max_surprise_weight: clamp_finite(self.max_surprise_weight, 1.0, MAX_SURPRISE_WEIGHT),
            max_decile_share: clamp_finite(self.max_decile_share, 0.0, MAX_DECILE_SHARE),
            seed: self.seed,
            split_version: self.split_version,
        }
    }
}

fn clamp_finite(value: f32, low: f32, high: f32) -> f32 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        low
    }
}

/// Stable identity of one training transition.
///
/// `transition_id` is the anchor row's `row_id` — the same key
/// `materialize.rs` joins verified labels on, so a transition, its label, and
/// its surprise all name the same thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionKey {
    pub transition_id: String,
    pub session_id: String,
    /// Guarded action this transition's anchor row belongs to, when it has one.
    pub action_attempt_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Whether a session belongs to the held-out partition.
///
/// The signature is the guarantee: surprise, replay counts, recency, labels and
/// model state are not arguments, so no amount of prioritisation can move a
/// session across the split. Only `split_version` can, and it is recorded on
/// every summary.
#[must_use]
pub fn is_held_out(session_id: &str, held_out_fraction: f32, split_version: u32) -> bool {
    let fraction = clamp_finite(held_out_fraction, 0.0, MAX_HELD_OUT_FRACTION);
    if fraction <= 0.0 {
        return false;
    }
    let cutoff = (f64::from(fraction) * SPLIT_BUCKETS as f64) as u64;
    split_bucket(session_id, split_version) < cutoff
}

fn split_bucket(session_id: &str, split_version: u32) -> u64 {
    let mut hash = fnv1a64(HELD_OUT_SPLIT_SALT.as_bytes(), 0xcbf2_9ce4_8422_2325);
    hash = fnv1a64(&split_version.to_be_bytes(), hash);
    hash = fnv1a64(b":", hash);
    fnv1a64(session_id.as_bytes(), hash) % SPLIT_BUCKETS
}

/// FNV-1a, chosen over `DefaultHasher` because the standard hasher's output is
/// explicitly unstable across releases; a corpus partition that re-shuffles on a
/// toolchain bump invalidates every evaluation window recorded under it.
fn fnv1a64(bytes: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Latent surprise per trace row, joined through the guarded action identity.
///
/// A row without an `action_attempt_id`, an outcome without a
/// `latent_surprise`, or a non-finite/negative value contributes nothing. That
/// is deliberate: absence of a scored prediction is not evidence of low
/// surprise, and a transition with no entry here is later given the *baseline*
/// weight rather than a zero one, so a missing outcome is never punished.
#[must_use]
pub fn surprise_by_row_id(
    rows: &[WorldTraceRow],
    outcomes: &[WorldGuardrailOutcome],
) -> BTreeMap<String, f32> {
    let mut by_action: BTreeMap<&str, f32> = BTreeMap::new();
    for outcome in outcomes {
        let Some(surprise) = outcome.latent_surprise else {
            continue;
        };
        if !surprise.is_finite() || surprise < 0.0 {
            continue;
        }
        // Ledgers are append-only and an action may be finalised more than
        // once; the last write is the current adjudication.
        by_action.insert(outcome.action_id.as_str(), surprise);
    }

    let mut by_row = BTreeMap::new();
    for row in rows {
        let Some(attempt) = row.action_attempt_id.as_deref() else {
            continue;
        };
        if let Some(surprise) = by_action.get(attempt) {
            by_row.insert(row.row_id.clone(), *surprise);
        }
    }
    by_row
}

/// SplitMix64 — a seeded, dependency-free, platform-stable PRNG.
///
/// Deterministic so a plan can be reproduced from `(seed, split_version)`
/// alone, which is what makes a matched baseline/canary pair replayable.
pub(super) struct SplitMix64(u64);

impl SplitMix64 {
    pub(super) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(super) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub(super) fn next_unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}
