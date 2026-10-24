//! The frozen stage a `verify-frozen-chain` call names.

use anyhow::{Result, anyhow};

/// The stage a `verify-frozen-chain` call re-verifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrozenStage {
    Acceptance,
    Skeleton,
}

impl FrozenStage {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        match text {
            "acceptance" => Ok(Self::Acceptance),
            "skeleton" => Ok(Self::Skeleton),
            other => Err(anyhow!(
                "verify-frozen-chain stage must be acceptance or skeleton, found '{other}'"
            )),
        }
    }

    pub(crate) fn command_id(self) -> &'static str {
        match self {
            Self::Acceptance => "verify-frozen-acceptance",
            Self::Skeleton => "verify-frozen-skeleton",
        }
    }
}
