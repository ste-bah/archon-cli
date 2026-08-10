//! Building one bounded replay batch and the record of what it did.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::policy::{PRIORITY_VERSION, ReplayPolicy, SplitMix64, TransitionKey, is_held_out};

/// Why a computed plan was not applied to the trainer's example set.
///
/// Every variant is a refusal, never a fallback that quietly half-applies: the
/// trainer either trains on the sampled subset or on everything it already had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplaySkipReason {
    /// `prioritized_enabled = false` — the default. Plan computed, not applied.
    Disabled,
    /// Nothing left after the held-out split.
    EmptyPool,
    /// No training transition carries a latent surprise value.
    ///
    /// Applying a batch cap with no priority signal would shrink the training
    /// set and buy nothing, so replay declines rather than narrowing blindly.
    NoSurpriseSignal,
    /// The key list and the example list disagree in length.
    ///
    /// The trainer builds examples and keys from the same window scan, so this
    /// means the two paths have drifted. Refusing is the only safe answer —
    /// applying a mismatched index set would train on transitions other than
    /// the ones that were scored.
    IndexMismatch,
}

impl ReplaySkipReason {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::EmptyPool => "empty_pool",
            Self::NoSurpriseSignal => "no_surprise_signal",
            Self::IndexMismatch => "index_mismatch",
        }
    }
}

/// One selected transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplaySample {
    /// Position in the transition list the trainer built its examples from.
    pub index: usize,
    pub transition_id: String,
    /// 0 = highest-priority tenth of the pool, 9 = lowest.
    pub priority_decile: u8,
    /// Sampling weight, in `[1, max_surprise_weight]`.
    pub weight: f32,
    /// `(1/n) / p_selected` — the correction that keeps a prioritised batch
    /// from silently redefining the training distribution.
    pub importance_weight: f32,
    pub prioritized: bool,
}

/// What one plan did, small enough to record on a training report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplaySummary {
    pub priority_version: String,
    pub split_version: u32,
    pub transitions: usize,
    pub pool: usize,
    pub held_out: usize,
    pub held_out_sessions: usize,
    /// Training transitions carrying a latent surprise value.
    pub with_surprise: usize,
    pub requested_batch: usize,
    pub selected: usize,
    pub prioritized_selected: usize,
    pub uniform_selected: usize,
    /// Largest share of the batch supplied by any one priority decile.
    pub max_decile_share: f32,
    pub min_importance_weight: f32,
    pub max_importance_weight: f32,
    pub applied: bool,
    pub skip_reason: Option<ReplaySkipReason>,
}

impl ReplaySummary {
    #[must_use]
    pub fn summary_line(&self) -> String {
        format!(
            "{} pool={} held_out={} ({} sessions) surprise={} batch={} \
             (prioritized {} / uniform {}) max_decile_share={:.2} \
             importance=[{:.2}, {:.2}] applied={}{}",
            self.priority_version,
            self.pool,
            self.held_out,
            self.held_out_sessions,
            self.with_surprise,
            self.selected,
            self.prioritized_selected,
            self.uniform_selected,
            self.max_decile_share,
            self.min_importance_weight,
            self.max_importance_weight,
            self.applied,
            self.skip_reason
                .map(|reason| format!(" ({})", reason.as_str()))
                .unwrap_or_default(),
        )
    }
}

/// A computed replay batch plus the partition it was drawn from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayPlan {
    pub summary: ReplaySummary,
    pub selected: Vec<ReplaySample>,
    /// Transition positions reserved for evaluation.
    ///
    /// Disjoint from `selected` by construction: the pool these were removed
    /// from is the only thing the sampler ever sees.
    pub held_out_indices: Vec<usize>,
}

impl ReplayPlan {
    #[must_use]
    pub fn applied(&self) -> bool {
        self.summary.applied
    }

    /// Selected positions, ascending, for slicing the trainer's example list.
    #[must_use]
    pub fn selected_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = self.selected.iter().map(|sample| sample.index).collect();
        indices.sort_unstable();
        indices
    }
}

/// Build a bounded replay batch over `keys`.
///
/// `expected_examples` is the length of the example list the trainer will slice.
/// It is checked rather than assumed: a mismatch yields
/// [`ReplaySkipReason::IndexMismatch`] and an unapplied plan.
#[must_use]
pub fn plan_replay(
    keys: &[TransitionKey],
    surprise: &BTreeMap<String, f32>,
    policy: ReplayPolicy,
    expected_examples: usize,
) -> ReplayPlan {
    let policy = policy.clamped();

    // Partition first. Everything below reads `pool` only, so no weighting code
    // can reach a held-out index.
    let mut pool = Vec::new();
    let mut held_out_indices = Vec::new();
    let mut held_out_sessions = BTreeSet::new();
    for (index, key) in keys.iter().enumerate() {
        if is_held_out(
            &key.session_id,
            policy.held_out_fraction,
            policy.split_version,
        ) {
            held_out_sessions.insert(key.session_id.as_str());
            held_out_indices.push(index);
        } else {
            pool.push(index);
        }
    }

    let weights = surprise_rank_weights(keys, &pool, surprise, policy.max_surprise_weight);
    let with_surprise = pool
        .iter()
        .filter(|index| surprise.contains_key(&keys[**index].transition_id))
        .count();
    let deciles = priority_deciles(keys, &pool, &weights);

    let selected = if pool.is_empty() {
        Vec::new()
    } else {
        draw(keys, &pool, &weights, &deciles, policy)
    };

    let skip = if keys.len() != expected_examples {
        Some(ReplaySkipReason::IndexMismatch)
    // `selected` cannot be empty while the pool is not, but the trainer would
    // train on nothing if it ever were, so the guard is on the observed batch
    // rather than on the argument that it is impossible.
    } else if pool.is_empty() || selected.is_empty() {
        Some(ReplaySkipReason::EmptyPool)
    } else if with_surprise == 0 {
        Some(ReplaySkipReason::NoSurpriseSignal)
    } else if !policy.prioritized_enabled {
        Some(ReplaySkipReason::Disabled)
    } else {
        None
    };

    let decile_share = observed_decile_share(&selected);
    let (min_importance, max_importance) = importance_range(&selected);
    let prioritized_selected = selected.iter().filter(|sample| sample.prioritized).count();

    ReplayPlan {
        summary: ReplaySummary {
            priority_version: PRIORITY_VERSION.to_string(),
            split_version: policy.split_version,
            transitions: keys.len(),
            pool: pool.len(),
            held_out: held_out_indices.len(),
            held_out_sessions: held_out_sessions.len(),
            with_surprise,
            requested_batch: policy.batch_size.min(pool.len()),
            selected: selected.len(),
            prioritized_selected,
            uniform_selected: selected.len() - prioritized_selected,
            max_decile_share: decile_share,
            min_importance_weight: min_importance,
            max_importance_weight: max_importance,
            applied: skip.is_none(),
            skip_reason: skip,
        },
        selected,
        held_out_indices,
    }
}

/// Weight per pool position, from the transition's *rank* among the
/// surprise-carrying transitions in the pool.
///
/// Transitions with no recorded surprise keep the baseline `1.0`: a missing
/// outcome is absence of evidence, not evidence of a small error, and giving it
/// zero weight would quietly delete every unscored transition from training.
fn surprise_rank_weights(
    keys: &[TransitionKey],
    pool: &[usize],
    surprise: &BTreeMap<String, f32>,
    max_weight: f32,
) -> Vec<f32> {
    let mut weights = vec![1.0_f32; pool.len()];
    let mut scored: Vec<(usize, f32)> = pool
        .iter()
        .enumerate()
        .filter_map(|(position, index)| {
            surprise
                .get(&keys[*index].transition_id)
                .map(|value| (position, *value))
        })
        .collect();
    if scored.is_empty() {
        return weights;
    }
    scored.sort_by(|left, right| {
        left.1
            .partial_cmp(&right.1)
            .unwrap_or(Ordering::Equal)
            // Ties broken on identity so the ranking is a function of the
            // corpus, not of iteration order.
            .then_with(|| {
                keys[pool[left.0]]
                    .transition_id
                    .cmp(&keys[pool[right.0]].transition_id)
            })
    });
    let count = scored.len() as f32;
    for (rank, (position, _)) in scored.iter().enumerate() {
        let percentile = (rank + 1) as f32 / count;
        weights[*position] = 1.0 + (max_weight - 1.0) * percentile;
    }
    weights
}

/// Decile per pool position: 0 is the highest-weight tenth.
fn priority_deciles(keys: &[TransitionKey], pool: &[usize], weights: &[f32]) -> Vec<u8> {
    let mut order: Vec<usize> = (0..pool.len()).collect();
    order.sort_by(|left, right| {
        weights[*right]
            .partial_cmp(&weights[*left])
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                keys[pool[*left]]
                    .transition_id
                    .cmp(&keys[pool[*right]].transition_id)
            })
    });
    let mut deciles = vec![0_u8; pool.len()];
    let count = pool.len().max(1);
    for (rank, position) in order.into_iter().enumerate() {
        deciles[position] = ((rank * 10) / count).min(9) as u8;
    }
    deciles
}

/// The bounded mixture draw: a uniform floor first, then weighted picks, with a
/// decile quota applied to both.
fn draw(
    keys: &[TransitionKey],
    pool: &[usize],
    weights: &[f32],
    deciles: &[u8],
    policy: ReplayPolicy,
) -> Vec<ReplaySample> {
    let count = pool.len();
    let total = policy.batch_size.min(count);
    let fraction = policy.prioritized_fraction;
    let uniform_target = (((1.0 - fraction) * total as f32).ceil() as usize).min(total);
    let prioritized_target = total - uniform_target;
    // At least one so a tiny batch is never unfillable by its own cap.
    let decile_quota = ((total as f32 * policy.max_decile_share).floor() as usize).max(1);

    let weight_total: f32 = weights.iter().sum();
    let mut rng = SplitMix64::new(policy.seed ^ u64::from(policy.split_version));
    let mut taken = vec![false; count];
    let mut decile_used = [0_usize; 10];
    let mut samples = Vec::with_capacity(total);

    let mut order: Vec<usize> = (0..count).collect();
    for position in (1..count).rev() {
        let swap = (rng.next_u64() % (position as u64 + 1)) as usize;
        order.swap(position, swap);
    }
    for position in order {
        if samples.len() >= uniform_target {
            break;
        }
        if take(
            &mut taken,
            &mut decile_used,
            deciles,
            decile_quota,
            position,
        ) {
            samples.push(sample(
                keys,
                pool,
                weights,
                deciles,
                position,
                weight_total,
                fraction,
                false,
            ));
        }
    }

    for _ in 0..prioritized_target {
        let available: f32 = (0..count)
            .filter(|position| {
                !taken[*position] && decile_used[deciles[*position] as usize] < decile_quota
            })
            .map(|position| weights[position])
            .sum();
        if available <= 0.0 {
            break;
        }
        let mut ticket = rng.next_unit() as f32 * available;
        let mut chosen = None;
        for position in 0..count {
            if taken[position] || decile_used[deciles[position] as usize] >= decile_quota {
                continue;
            }
            ticket -= weights[position];
            if ticket <= 0.0 {
                chosen = Some(position);
                break;
            }
        }
        let Some(position) = chosen else {
            break;
        };
        if take(
            &mut taken,
            &mut decile_used,
            deciles,
            decile_quota,
            position,
        ) {
            samples.push(sample(
                keys,
                pool,
                weights,
                deciles,
                position,
                weight_total,
                fraction,
                true,
            ));
        }
    }

    samples
}

fn take(
    taken: &mut [bool],
    decile_used: &mut [usize; 10],
    deciles: &[u8],
    quota: usize,
    position: usize,
) -> bool {
    let decile = deciles[position] as usize;
    if taken[position] || decile_used[decile] >= quota {
        return false;
    }
    taken[position] = true;
    decile_used[decile] += 1;
    true
}

#[allow(clippy::too_many_arguments)]
fn sample(
    keys: &[TransitionKey],
    pool: &[usize],
    weights: &[f32],
    deciles: &[u8],
    position: usize,
    weight_total: f32,
    fraction: f32,
    prioritized: bool,
) -> ReplaySample {
    let count = pool.len() as f32;
    // p_i = (1 - f)/n + f * w_i / W, so 1/(n * p_i) is the correction. The
    // uniform floor keeps the denominator at or above (1 - f), which is what
    // caps this at 1/(1 - f) instead of letting it run away.
    let denominator = if weight_total > 0.0 {
        (1.0 - fraction) + fraction * count * weights[position] / weight_total
    } else {
        1.0
    };
    ReplaySample {
        index: pool[position],
        transition_id: keys[pool[position]].transition_id.clone(),
        priority_decile: deciles[position],
        weight: weights[position],
        importance_weight: if denominator > 0.0 {
            1.0 / denominator
        } else {
            1.0
        },
        prioritized,
    }
}

fn observed_decile_share(samples: &[ReplaySample]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut counts = [0_usize; 10];
    for sample in samples {
        counts[sample.priority_decile as usize] += 1;
    }
    counts.into_iter().max().unwrap_or(0) as f32 / samples.len() as f32
}

fn importance_range(samples: &[ReplaySample]) -> (f32, f32) {
    let mut low = f32::INFINITY;
    let mut high = 0.0_f32;
    for sample in samples {
        low = low.min(sample.importance_weight);
        high = high.max(sample.importance_weight);
    }
    if samples.is_empty() {
        (0.0, 0.0)
    } else {
        (low, high)
    }
}
