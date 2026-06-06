use serde::{Deserialize, Serialize};

/// Two-person approval workflow for sensitive trading actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MakerCheckerApproval {
    pub request_id: String,
    pub maker: String,
    pub checker: String,
    pub action: String,
    pub approved: bool,
    pub rationale: String,
}

impl MakerCheckerApproval {
    pub fn new(
        request_id: impl Into<String>,
        maker: impl Into<String>,
        checker: impl Into<String>,
        action: impl Into<String>,
        approved: bool,
        rationale: impl Into<String>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            maker: maker.into(),
            checker: checker.into(),
            action: action.into(),
            approved,
            rationale: rationale.into(),
        }
    }

    pub fn verify_pair(&self) -> Result<(), MakerCheckerError> {
        if self.maker.trim().is_empty() || self.checker.trim().is_empty() {
            return Err(MakerCheckerError::MissingActor);
        }
        if self.maker == self.checker {
            return Err(MakerCheckerError::SameActor);
        }
        if !self.approved {
            return Err(MakerCheckerError::NotApproved);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MakerCheckerError {
    MissingActor,
    SameActor,
    NotApproved,
}

impl MakerCheckerError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingActor => "ERR-MAKER-CHECKER-MISSING-ACTOR",
            Self::SameActor => "ERR-MAKER-CHECKER-SAME-ACTOR",
            Self::NotApproved => "ERR-MAKER-CHECKER-NOT-APPROVED",
        }
    }
}

impl std::fmt::Display for MakerCheckerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for MakerCheckerError {}

#[cfg(test)]
mod tests {
    use super::{MakerCheckerApproval, MakerCheckerError};

    #[test]
    fn maker_and_checker_must_be_distinct() {
        let approval = MakerCheckerApproval::new("r1", "alice", "alice", "raise-limit", true, "ok");
        assert_eq!(approval.verify_pair(), Err(MakerCheckerError::SameActor));
    }

    #[test]
    fn approved_distinct_pair_verifies() {
        let approval = MakerCheckerApproval::new("r1", "alice", "bob", "raise-limit", true, "ok");
        assert!(approval.verify_pair().is_ok());
    }
}
