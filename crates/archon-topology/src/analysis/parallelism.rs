//! Achievable concurrency per wave, against the declared budget.

use crate::error::TopologyError;
use crate::ir::TaskGraph;

/// How much of the declared parallelism budget the graph's shape can actually
/// use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelismProfile {
    /// Node count per wave, in wave order.
    pub wave_widths: Vec<usize>,
    /// Widest wave. This is the most concurrency the graph can ever achieve,
    /// regardless of budget.
    pub peak_width: usize,
    /// `budget.max_parallelism`, repeated so a caller holding only the profile
    /// can render the comparison.
    pub budget_max_parallelism: u32,
    /// Slots the budget reserves that no wave is wide enough to fill.
    ///
    /// Non-zero means the graph reserves capacity it can never use. Zero when
    /// the budget is at or below `peak_width`.
    pub unusable_slots: u32,
    /// Waves wider than the budget, as `(wave index, width)`. These serialize
    /// against the cap at runtime, so the graph's real span exceeds
    /// `wave_widths.len()`.
    pub budget_limited_waves: Vec<(usize, usize)>,
}

impl ParallelismProfile {
    /// True when the budget reserves concurrency the graph cannot use.
    #[must_use]
    pub fn over_provisioned(&self) -> bool {
        self.unusable_slots > 0
    }

    /// True when at least one wave is wider than the budget allows.
    #[must_use]
    pub fn budget_limited(&self) -> bool {
        !self.budget_limited_waves.is_empty()
    }
}

impl TaskGraph {
    /// Maximum achievable concurrency per wave versus `budget.max_parallelism`.
    pub fn parallelism_profile(&self) -> Result<ParallelismProfile, TopologyError> {
        let waves = self.waves()?;
        let wave_widths: Vec<usize> = waves.iter().map(Vec::len).collect();
        let peak_width = wave_widths.iter().copied().max().unwrap_or(0);
        let budget = self.budget.max_parallelism;

        let unusable_slots =
            u32::try_from(peak_width).map_or(0, |peak| budget.saturating_sub(peak));

        let budget_limited_waves = wave_widths
            .iter()
            .enumerate()
            .filter(|&(_, &width)| u32::try_from(width).is_ok_and(|width| width > budget))
            .map(|(wave, &width)| (wave, width))
            .collect();

        Ok(ParallelismProfile {
            wave_widths,
            peak_width,
            budget_max_parallelism: budget,
            unusable_slots,
            budget_limited_waves,
        })
    }
}
