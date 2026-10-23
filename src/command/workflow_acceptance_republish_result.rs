use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReauthorResult {
    pub(crate) acceptance_digest: String,
    pub(crate) freeze_event_id: String,
    pub(crate) skeleton_digest: Option<String>,
    pub(crate) diagnostics: Vec<String>,
    /// Checks still pending, with their latest host or authored finding.
    pub(crate) unproven: BTreeMap<String, String>,
    /// The subset pending because their authored check or judge reply needs
    /// another attempt. These findings do not represent host faults.
    pub(crate) authored_pending: BTreeMap<String, String>,
    pub(crate) published: bool,
}
