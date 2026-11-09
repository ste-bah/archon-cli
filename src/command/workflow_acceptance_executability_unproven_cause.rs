/// Why the host could not prove a check (F6). Only a program that neither
/// the check's site nor the host has is the author's to replace; a site
/// that lacks what the host has is a defect of the site the host built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnprovenCause {
    /// The host's: its site, its environment or its run. A strike makes
    /// the same failure its author's after a resume (Issue 328, #356).
    Host,
    /// Validation, corruption, or an unclassified host diagnostic.
    PermanentHost,
    /// An inconclusive host probe: retried without a strike or author finding.
    TransientHost,
    /// The check's site lacks a program (or a tool's subcommand) the host
    /// has, or the host could not tell: a defect of the site the host
    /// built, so no strike ever makes it the author's (F6).
    /// Issue 548: no host repair makes the site provide it, so it goes to
    /// its author at once, in the same budget, with no strike saved.
    SiteLacks,
    /// The check runs a program neither its site nor the host provides.
    /// Issue 557: no host action makes it available, so it goes to its
    /// author at once on the probe path too, with no strike saved.
    ProgramAbsent,
}

impl UnprovenCause {
    pub(crate) fn from_class(class: crate::command::workflow_host_fault::Class) -> Self {
        match class {
            crate::command::workflow_host_fault::Class::Transient => Self::Host,
            _ => Self::PermanentHost,
        }
    }
    pub(crate) fn class(self) -> crate::command::workflow_host_fault::Class {
        match self {
            Self::Host | Self::TransientHost | Self::SiteLacks => {
                crate::command::workflow_host_fault::Class::Transient
            }
            _ => crate::command::workflow_host_fault::Class::Permanent,
        }
    }
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Host | Self::TransientHost => "host_transient",
            Self::SiteLacks => "host_site_lacks",
            Self::ProgramAbsent => "host_program_absent",
            Self::PermanentHost => "host_permanent",
        }
    }
}
