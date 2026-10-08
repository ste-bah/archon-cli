//! The re-author stall outcome (Issue 288): no progress on `pending` for
//! [`super::REAUTHOR_ATTEMPTS`] attempts.

#[derive(Debug)]
pub(crate) struct ReauthorStalled {
    pub(super) report: String,
    pub(crate) pending: Vec<String>,
}

impl std::fmt::Display for ReauthorStalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pending = self.pending.join(", ");
        write!(f, "{}\nstill pending: {pending}", self.report)
    }
}

impl std::error::Error for ReauthorStalled {}
