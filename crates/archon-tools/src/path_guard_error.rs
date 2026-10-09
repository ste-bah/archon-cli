use std::fmt;
use std::ops::Deref;

use crate::tool::ToolResult;

#[derive(Debug)]
pub(crate) struct GuardError {
    message: String,
    refusal: bool,
}

impl GuardError {
    pub(crate) fn ordinary(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            refusal: false,
        }
    }

    pub(crate) fn refusal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            refusal: true,
        }
    }

    pub(crate) fn into_tool_result(self) -> ToolResult {
        if self.refusal {
            ToolResult::refusal(self.message)
        } else {
            ToolResult::error(self.message)
        }
    }
}

impl From<String> for GuardError {
    fn from(message: String) -> Self {
        Self::ordinary(message)
    }
}

impl From<GuardError> for String {
    fn from(error: GuardError) -> Self {
        error.message
    }
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Deref for GuardError {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}
