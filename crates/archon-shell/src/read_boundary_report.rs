use std::fmt;

/// Prepared launch diagnostics with an outcome read from the child after spawn.
#[derive(Debug)]
pub struct LaunchReport {
    pub(super) prepared: String,
    #[cfg(target_os = "linux")]
    pub(super) seals: Option<crate::write_boundary::landlock::SealStatus>,
}

impl From<String> for LaunchReport {
    fn from(prepared: String) -> Self {
        Self {
            prepared,
            #[cfg(target_os = "linux")]
            seals: None,
        }
    }
}

impl From<&str> for LaunchReport {
    fn from(prepared: &str) -> Self {
        prepared.to_owned().into()
    }
}

impl LaunchReport {
    pub fn contains(&self, text: &str) -> bool {
        self.to_string().contains(text)
    }
}

impl fmt::Display for LaunchReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(target_os = "linux")]
        if let Some(seals) = &self.seals {
            return write!(formatter, "{}; {}", self.prepared, seals.report());
        }
        formatter.write_str(&self.prepared)
    }
}
