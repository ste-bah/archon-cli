//! The score placeholder. Read this before trusting a number out of R7.
//!
//! # There is no calibration here, on purpose
//!
//! The four stores report incomparable quantities. `archon-knowledge`'s hybrid
//! retriever emits a weighted sum of a term-overlap ratio and `1 - distance/2`.
//! `archon-docs` emits its own weighted sum with different weights. The code
//! index emits a cosine similarity. `archon-memory`'s `recall_memories` emits
//! **nothing at all** — it returns an ordered `Vec<Memory>` with the score
//! discarded inside the crate.
//!
//! That last fact settles the design. A fusion that reads raw scores cannot
//! include memory without inventing one for it, and the R7 gate scores source
//! coverage at >= 0.95, so dropping memory is not available either.
//!
//! Min-max scaling per source would produce comparable-looking numbers with no
//! basis: it maps whatever came back to `[0, 1]`, so a query where every source
//! returned junk and one where every source returned gold are reported
//! identically. The roadmap says so directly at line 286 — "measured
//! calibration, not ad hoc min-max scaling".
//!
//! # What this does instead
//!
//! Reciprocal rank. The score is a function of position alone, so it makes
//! exactly one claim — "this source ranked this hit above that one" — which the
//! source did in fact assert. It makes no cross-source claim, because it cannot
//! tell two sources apart. Rank *k* from memory and rank *k* from docs get the
//! same number, and the merge falls back to the deterministic tie-break in
//! [`crate::recall::RecallSource`].
//!
//! This is a placeholder, not a result. It is not tuned, was not fitted to
//! anything, and `k` was not chosen by measurement. Promoting R7 requires
//! replacing it with a score fitted on the corpus in roadmap line 306 — 500
//! replayable queries with adjudicated relevant sources, >= 50 per source — and
//! tagging hits [`ScoreCalibration::Measured`] so the difference is visible in
//! the output rather than in a changelog.
//!
//! [`ScoreCalibration::Measured`]: crate::recall::ScoreCalibration::Measured

use crate::recall::ScoreCalibration;

/// Rank offset in the reciprocal-rank placeholder.
///
/// 60 is the value the reciprocal-rank-fusion literature uses by convention. It
/// was NOT measured here and carries no authority in this repository; it is
/// stated as a constant so that the arbitrary part of the placeholder is one
/// named number rather than a scattering of magic in the merge.
pub const UNCALIBRATED_RANK_K: f32 = 60.0;

/// Human-readable description of the placeholder, for logs and CLI output.
///
/// Says "uncalibrated" in the string itself: this text reaches operators, and an
/// operator who sees a score column with no such warning will read it as
/// relevance.
pub const UNCALIBRATED_METHOD: &str =
    "uncalibrated reciprocal rank (k=60), within-source order only";

/// Score for a zero-based rank within one source.
///
/// Strictly decreasing in `rank`, and in `(0, 1]` so it is comfortable to
/// display. Neither property is evidence of anything.
pub fn uncalibrated_rank_score(rank: usize) -> f32 {
    // `rank as f32` saturates at f32::MAX for absurd ranks rather than wrapping,
    // so the score stays positive and monotone for any input a store could
    // plausibly return.
    UNCALIBRATED_RANK_K / (UNCALIBRATED_RANK_K + rank as f32)
}

/// The calibration tag every hit built by this crate carries.
pub fn calibration() -> ScoreCalibration {
    ScoreCalibration::UncalibratedRankOrder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_zero_scores_one() {
        assert_eq!(uncalibrated_rank_score(0), 1.0);
    }

    #[test]
    fn score_strictly_decreases_with_rank() {
        let mut previous = uncalibrated_rank_score(0);
        for rank in 1..64 {
            let score = uncalibrated_rank_score(rank);
            assert!(
                score < previous,
                "rank {rank} scored {score}, not below {previous}"
            );
            assert!(score > 0.0);
            previous = score;
        }
    }

    /// The placeholder is source-blind by construction. If this ever fails,
    /// something has started encoding a cross-source preference without a
    /// corpus to justify it.
    #[test]
    fn score_depends_on_rank_alone() {
        assert_eq!(uncalibrated_rank_score(3), uncalibrated_rank_score(3));
        assert_eq!(uncalibrated_rank_score(7), UNCALIBRATED_RANK_K / 67.0);
    }

    #[test]
    fn calibration_is_never_measured() {
        assert!(!calibration().is_measured());
    }

    #[test]
    fn method_text_warns_the_reader() {
        assert!(UNCALIBRATED_METHOD.contains("uncalibrated"));
    }
}
