//! The **declared permission format** and its mapping onto [`PermissionClass`].
//!
//! # Why this exists
//!
//! Milestone 1 shipped with no schema at all. `WorkflowSpec::permissions` is
//! typed `BTreeMap<String, serde_json::Value>`, `deserialize_permissions`
//! accepts any JSON object verbatim, and nothing in the tree read the result —
//! so the lowering guessed at plausible shapes and fell through to
//! [`PermissionClass::Safe`] on anything it did not recognise. That is
//! fail-open, and milestone 3 turns this field into the input of a *safety*
//! invariant. A safety invariant cannot be grounded in a guess.
//!
//! # The grounding
//!
//! It is grounded in the enum that already exists and that admission already
//! keys off at runtime: [`archon_tools::tool::PermissionLevel`]
//! (`crates/archon-tools/src/tool.rs`). Every tool in the tree already declares
//! one of its three values for every invocation, and the tool-run admission
//! callback already receives it. Inventing a fourth vocabulary for authored
//! specs would mean two ladders that could disagree.
//!
//! ```text
//! declared      PermissionLevel     PermissionClass
//! ---------     ---------------     ---------------
//! "safe"        Safe                Safe
//! "risky"       Risky               Risky
//! "dangerous"   Dangerous           Irreversible
//! ```
//!
//! `Dangerous → Irreversible` rather than `→ Risky`: milestone 3 gates on
//! irreversibility, and under-classifying there is the failure that matters.
//! The command classifier's `Dangerous` set (`git push`, `rm -rf`, `sudo`,
//! `shutdown`) is exactly the design's own list of irreversible effects —
//! "push, deploy, publish, force-delete".
//!
//! `archon-topology` cannot name `PermissionLevel` directly: `archon-tools` is
//! a tokio-and-network crate and the dependency budget for this crate is
//! petgraph + serde. The two are therefore tied together by
//! `archon_core::orchestrator::topology::permission_class_for_level`, which is
//! the one runtime mapping, plus the conformance test beside it that asserts
//! this parser and that mapping agree variant for variant. A new
//! `PermissionLevel` variant fails that test.
//!
//! # Authoring
//!
//! In a `WorkflowSpec`, `permissions` maps a stage id — or the blanket key
//! `default` (alias `*`) — to a level:
//!
//! ```yaml
//! permissions:
//!   default: safe
//!   deploy: dangerous
//!   review: { level: risky }
//! ```
//!
//! The value is either the level as a string or an object carrying it under
//! `level` (canonical) or `class` / `permission` (accepted aliases, kept
//! because they were the shapes milestone 1's guesswork already read).
//! Matching is case-insensitive and surrounding whitespace is trimmed.
//!
//! `irreversible` is accepted as an alias of `dangerous`, because that is what
//! the resulting [`PermissionClass`] is called and an author reading the IR
//! will reach for it. `dangerous` is canonical.
//!
//! # Unrecognised values stay fail-open
//!
//! Anything not in the table lowers to [`PermissionClass::Safe`], not to the
//! strictest class. Milestone 3's rule is that enforcement must never fail
//! closed on a bookkeeping gap, and a typo in a permissions map is a
//! bookkeeping gap. The cost is that a typo silently disarms the gate for that
//! stage, which is why [`is_declared_permission`] exists: a validator that
//! wants to reject typos at authoring time — where failing loudly is free —
//! can ask.

use crate::ir::PermissionClass;

/// The canonical declared levels, in increasing severity.
///
/// These are the lowercase names of the [`archon_tools::tool::PermissionLevel`]
/// variants. Aliases accepted by [`PermissionClass::from_declared`] are
/// deliberately absent — this is the list to *print* in an error message.
pub const DECLARED_PERMISSION_LEVELS: [&str; 3] = ["safe", "risky", "dangerous"];

impl PermissionClass {
    /// Parse a declared permission level.
    ///
    /// Case-insensitive, whitespace-trimmed. Returns `None` for anything
    /// outside the table above, leaving the fail-open decision to the caller
    /// rather than making it here.
    #[must_use]
    pub fn from_declared(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "safe" => Some(PermissionClass::Safe),
            "risky" => Some(PermissionClass::Risky),
            // `irreversible` is the IR's name for the same thing; accepted so
            // an author reading the IR can write what they see.
            "dangerous" | "irreversible" => Some(PermissionClass::Irreversible),
            _ => None,
        }
    }

    /// The canonical declared spelling of this class.
    ///
    /// Round-trips through [`PermissionClass::from_declared`].
    #[must_use]
    pub fn as_declared(self) -> &'static str {
        match self {
            PermissionClass::Safe => "safe",
            PermissionClass::Risky => "risky",
            PermissionClass::Irreversible => "dangerous",
        }
    }
}

/// Whether `raw` is a value the declared format recognises.
///
/// For validators. Runtime lowering must not branch on this — it falls open to
/// [`PermissionClass::Safe`] instead — but a spec validator running at
/// authoring time can and should reject an unrecognised level, because that is
/// the one place where failing loudly costs nothing.
#[must_use]
pub fn is_declared_permission(raw: &str) -> bool {
    PermissionClass::from_declared(raw).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_canonical_level_parses_and_round_trips() {
        for level in DECLARED_PERMISSION_LEVELS {
            let class = PermissionClass::from_declared(level).expect("canonical level parses");
            assert_eq!(
                PermissionClass::from_declared(class.as_declared()),
                Some(class),
                "{level} must round-trip through as_declared"
            );
        }
    }

    #[test]
    fn dangerous_is_irreversible_not_risky() {
        assert_eq!(
            PermissionClass::from_declared("dangerous"),
            Some(PermissionClass::Irreversible)
        );
        assert_eq!(
            PermissionClass::from_declared("DANGEROUS "),
            Some(PermissionClass::Irreversible)
        );
        assert_eq!(
            PermissionClass::from_declared("irreversible"),
            Some(PermissionClass::Irreversible)
        );
    }

    #[test]
    fn unrecognised_values_are_none_so_the_caller_owns_the_fallback() {
        for raw in ["", "  ", "dangerus", "critical", "17", "high"] {
            assert_eq!(PermissionClass::from_declared(raw), None, "{raw:?}");
            assert!(!is_declared_permission(raw), "{raw:?}");
        }
    }
}
