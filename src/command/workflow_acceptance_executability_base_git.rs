//! Keep inherited configuration and filters out of isolated source copies.
use super::*;
use archon_workflow::isolated_copy::{CopyError, IsolatedGit as SharedIsolation};

pub(super) struct IsolatedGit(SharedIsolation);
fn unrun(error: CopyError) -> Unrun {
    match error {
        CopyError::Io { error, context } => Unrun::error(error.into(), context),
        CopyError::Operation(text) => Unrun::unknown(text),
    }
}
impl IsolatedGit {
    pub(super) fn new(root: &Path) -> Result<Self, Unrun> {
        SharedIsolation::new(root).map(Self).map_err(unrun)
    }
    pub(super) fn template(&self) -> &Path {
        self.0.template()
    }
    pub(super) fn run(&self, args: &[&std::ffi::OsStr]) -> Result<(), Unrun> {
        self.0.run(args).map_err(unrun)
    }
}
