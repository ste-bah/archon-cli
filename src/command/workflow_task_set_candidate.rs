//! Who a freeze failure belongs to: the authored candidate, or the host.

use anyhow::Result;

/// Marks a failure caused by the authored candidate rather than by the host.
///
/// The two need different endings. A host problem stops the run; an artifact
/// problem is handed back to its author, who gets another attempt. Without the
/// distinction every authoring mistake — an id the PRD never defined, a missing
/// field — ended the decomposition on its first occurrence.
#[derive(Debug)]
pub(crate) struct CandidateRejected;

impl CandidateRejected {
    pub(crate) fn tag<T>(result: Result<T>) -> Result<T> {
        result.map_err(|error| error.context(Self))
    }

    pub(crate) fn caused(error: &anyhow::Error) -> bool {
        // `chain()` yields anyhow's own context wrapper, whose concrete type is
        // not this marker, so `is::<Self>()` never matched and every rejection
        // was reported as a host failure. Downcasting is what reaches a context
        // value.
        error.downcast_ref::<Self>().is_some()
    }
}

impl std::fmt::Display for CandidateRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("candidate artifact rejected")
    }
}

impl std::error::Error for CandidateRejected {}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn a_tagged_failure_is_recognised_through_later_context() {
        let tagged = CandidateRejected::tag::<()>(Err(anyhow!("id 'A1' is not defined")))
            .expect_err("the tag preserves the failure")
            .context("while staging freeze-acceptance");

        assert!(CandidateRejected::caused(&tagged));
        assert!(format!("{tagged:#}").contains("id 'A1' is not defined"));
    }

    #[test]
    fn an_untagged_failure_stays_a_host_failure() {
        assert!(!CandidateRejected::caused(&anyhow!(
            "judge response was truncated"
        )));
    }
}
