//! What each agent has actually looked at on disk (#193 Phase A).
//!
//! `Edit` re-reads a file immediately before writing it, so the replacement is
//! applied to current bytes and there is no torn read. The problem is upstream
//! of that: the model chose `old_string` from a view of the file that may be
//! arbitrarily old. If anything changed the file since — another agent in a
//! parallel session, a `git checkout`, a formatter, someone in their editor —
//! the match can land in a region that no longer means what the model believed,
//! and the edit succeeds silently. Nothing in the system could detect it.
//!
//! Archon makes that likelier than most: `execute-plan` runs a fresh subagent
//! per task and several sessions routinely share one tree.
//!
//! This module records observations and answers questions about them. It
//! decides nothing — whether an unobserved or stale write is refused, warned
//! about, or ignored is the caller's business, which is what keeps the policy
//! removable. Take the policy away and an unconstrained filesystem is left
//! behind, working exactly as it does today.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

/// An opaque handle to one version of a path.
///
/// Deliberately not a struct of fields. Locally it is built from mtime and
/// length; a backend over ssh or a container could use an etag or a content
/// hash instead, and nothing outside this module may depend on which. Compare
/// two of them for equality — that is the whole contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersion(String);

impl FileVersion {
    /// Read the current version of `path`, or `None` when it does not exist.
    ///
    /// An unreadable path is `None` too. "I could not see it" and "it was not
    /// there" are the same evidence for this purpose: neither is grounds for
    /// believing a later edit is applied to what the model read.
    #[must_use]
    pub fn current(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or_else(|| "?".to_string(), |d| d.as_nanos().to_string());
        Some(Self(format!("{}:{modified}", meta.len())))
    }
}

/// What an agent saw when it last looked at a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// It was there, at this version.
    Present(FileVersion),
    /// It was checked for and was not there.
    ///
    /// A real observation, not the absence of one: a later create should be
    /// held to it, because the agent that decided to create the file did so
    /// believing nothing was in the way.
    Absent,
}

/// The answer to "may this agent write here on the strength of what it saw".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The observation matches what is on disk now.
    Fresh,
    /// This agent has never looked at the path.
    Unobserved,
    /// It looked, and what is there now is not what it saw.
    Stale {
        /// What changed, in words fit for a tool-result message.
        detail: String,
    },
}

/// Which agent an observation belongs to.
///
/// `session_id` alone will not do: it is copied verbatim from parent to child,
/// so keying on it would let a subagent inherit its parent's reading as
/// evidence for a file it never opened. `subagent_id` is `None` for the
/// top-level agent, which is a real answer rather than missing data.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Observer {
    pub session_id: String,
    pub subagent_id: Option<String>,
}

impl Observer {
    #[must_use]
    pub fn new(session_id: &str, subagent_id: Option<&str>) -> Self {
        Self {
            session_id: session_id.to_string(),
            subagent_id: subagent_id.map(str::to_string),
        }
    }
}

/// Per-agent record of observed paths.
///
/// Not persisted, and cleared when the session ends. A token from last week is
/// not evidence about now, and reloading one would be worse than having none:
/// it would answer "yes, fresh" for a file nobody in this process has read.
#[derive(Default)]
pub struct ObservationRegistry {
    seen: Mutex<HashMap<Observer, HashMap<PathBuf, Observation>>>,
}

/// Process-global registry, one entry per agent.
pub static FILE_OBSERVATIONS: LazyLock<ObservationRegistry> =
    LazyLock::new(ObservationRegistry::default);

impl ObservationRegistry {
    /// Record what is at `path` right now.
    ///
    /// Called after a tool has genuinely shown the agent those bytes. A missing
    /// file records [`Observation::Absent`] rather than nothing.
    pub fn record(&self, observer: &Observer, path: &Path) {
        let observation =
            FileVersion::current(path).map_or(Observation::Absent, Observation::Present);
        self.record_as(observer, path, observation);
    }

    /// Record a specific observation, for callers that already have one.
    pub fn record_as(&self, observer: &Observer, path: &Path, observation: Observation) {
        let key = normalise(path);
        if let Ok(mut seen) = self.seen.lock() {
            seen.entry(observer.clone())
                .or_default()
                .insert(key, observation);
        }
    }

    /// What this agent knows about `path`, if anything.
    #[must_use]
    pub fn observation(&self, observer: &Observer, path: &Path) -> Option<Observation> {
        let key = normalise(path);
        let seen = self.seen.lock().ok()?;
        seen.get(observer)?.get(&key).cloned()
    }

    /// Judge a pending write against what this agent has seen.
    #[must_use]
    pub fn verdict(&self, observer: &Observer, path: &Path) -> Verdict {
        let Some(observation) = self.observation(observer, path) else {
            return Verdict::Unobserved;
        };
        let current = FileVersion::current(path);
        match (observation, current) {
            (Observation::Present(seen), Some(now)) if seen == now => Verdict::Fresh,
            (Observation::Present(_), Some(_)) => Verdict::Stale {
                detail: "it has been modified since you read it".to_string(),
            },
            (Observation::Present(_), None) => Verdict::Stale {
                detail: "it has been deleted since you read it".to_string(),
            },
            (Observation::Absent, None) => Verdict::Fresh,
            (Observation::Absent, Some(_)) => Verdict::Stale {
                detail: "it did not exist when you checked and it does now".to_string(),
            },
        }
    }

    /// Drop everything this agent observed.
    ///
    /// Called when a session ends. Subagents are dropped with their parent
    /// session because their ids are only meaningful inside it.
    pub fn forget_session(&self, session_id: &str) {
        if let Ok(mut seen) = self.seen.lock() {
            seen.retain(|observer, _| observer.session_id != session_id);
        }
    }

    /// How many paths this agent has observed. For tests and diagnostics.
    #[must_use]
    pub fn len(&self, observer: &Observer) -> usize {
        self.seen
            .lock()
            .ok()
            .and_then(|seen| seen.get(observer).map(HashMap::len))
            .unwrap_or(0)
    }

    #[must_use]
    pub fn is_empty(&self, observer: &Observer) -> bool {
        self.len(observer) == 0
    }
}

/// Key a path by its canonical directory plus its file name.
///
/// `./src/lib.rs` and `src/lib.rs` are the same file, and an agent that read
/// one and edits the other has still read it — so the key has to survive
/// spelling.
///
/// It must also survive the file's *existence*, which the obvious
/// implementation does not: canonicalising the whole path fails once the file
/// is gone, so an observation recorded while it was present could not be found
/// after it was deleted, and a negative observation recorded while it was
/// absent could not be found once it appeared. Both are precisely the cases
/// this module exists to catch, and both silently reported "never looked".
/// Caught by the tests below, which is what they are for.
///
/// The directory is the part that reliably exists, so it carries the
/// canonicalisation and the file name is appended verbatim.
fn normalise(path: &Path) -> PathBuf {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    };
    match parent.canonicalize() {
        Ok(dir) => dir.join(name),
        Err(_) => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observer(name: &str) -> Observer {
        Observer::new(name, None)
    }

    #[test]
    fn a_path_nobody_looked_at_is_unobserved() {
        let registry = ObservationRegistry::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "fn main() {}").expect("write");

        assert_eq!(registry.verdict(&observer("s"), &file), Verdict::Unobserved);
    }

    #[test]
    fn an_unchanged_file_reads_fresh() {
        let registry = ObservationRegistry::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "fn main() {}").expect("write");

        registry.record(&observer("s"), &file);

        assert_eq!(registry.verdict(&observer("s"), &file), Verdict::Fresh);
    }

    /// The failure this exists to catch: something else changed the file
    /// between the read and the write.
    #[test]
    fn an_externally_modified_file_reads_stale() {
        let registry = ObservationRegistry::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "fn main() {}").expect("write");
        registry.record(&observer("s"), &file);

        std::fs::write(&file, "fn main() { changed_underneath() }").expect("rewrite");

        match registry.verdict(&observer("s"), &file) {
            Verdict::Stale { detail } => assert!(detail.contains("modified"), "{detail}"),
            other => panic!("expected stale, got {other:?}"),
        }
    }

    #[test]
    fn a_deleted_file_reads_stale() {
        let registry = ObservationRegistry::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "x").expect("write");
        registry.record(&observer("s"), &file);
        std::fs::remove_file(&file).expect("remove");

        match registry.verdict(&observer("s"), &file) {
            Verdict::Stale { detail } => assert!(detail.contains("deleted"), "{detail}"),
            other => panic!("expected stale, got {other:?}"),
        }
    }

    /// "I checked and it was not there" is evidence, and a file appearing
    /// since contradicts it.
    #[test]
    fn a_file_that_appeared_after_a_negative_observation_reads_stale() {
        let registry = ObservationRegistry::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("new.rs");

        registry.record(&observer("s"), &file);
        assert_eq!(
            registry.observation(&observer("s"), &file),
            Some(Observation::Absent)
        );
        assert_eq!(registry.verdict(&observer("s"), &file), Verdict::Fresh);

        std::fs::write(&file, "someone else made it").expect("write");

        match registry.verdict(&observer("s"), &file) {
            Verdict::Stale { detail } => assert!(detail.contains("did not exist"), "{detail}"),
            other => panic!("expected stale, got {other:?}"),
        }
    }

    /// A parent's read is not evidence for a child that never looked.
    #[test]
    fn a_subagent_does_not_inherit_its_parents_observations() {
        let registry = ObservationRegistry::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "x").expect("write");

        let parent = Observer::new("session-1", None);
        let child = Observer::new("session-1", Some("agent-7"));
        registry.record(&parent, &file);

        assert_eq!(registry.verdict(&parent, &file), Verdict::Fresh);
        assert_eq!(
            registry.verdict(&child, &file),
            Verdict::Unobserved,
            "session_id is copied verbatim to children, so keying on it alone \
             would hand a subagent evidence it never gathered"
        );
    }

    #[test]
    fn two_spellings_of_one_path_are_the_same_observation() {
        let registry = ObservationRegistry::default();
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("src")).expect("mkdir");
        let file = dir.path().join("src").join("a.rs");
        std::fs::write(&file, "x").expect("write");

        registry.record(&observer("s"), &file);
        let indirect = dir.path().join("src").join(".").join("a.rs");

        assert_eq!(registry.verdict(&observer("s"), &indirect), Verdict::Fresh);
    }

    #[test]
    fn ending_a_session_forgets_it_and_its_subagents() {
        let registry = ObservationRegistry::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "x").expect("write");

        let parent = Observer::new("session-1", None);
        let child = Observer::new("session-1", Some("agent-7"));
        let other = Observer::new("session-2", None);
        registry.record(&parent, &file);
        registry.record(&child, &file);
        registry.record(&other, &file);

        registry.forget_session("session-1");

        assert!(registry.is_empty(&parent));
        assert!(registry.is_empty(&child));
        assert_eq!(registry.len(&other), 1, "another session was collateral");
    }

    /// The token is compared, never parsed. This pins that two different files
    /// do not collide on it, without asserting anything about its shape.
    #[test]
    fn versions_of_different_content_differ() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "short").expect("write");
        let first = FileVersion::current(&file).expect("present");
        std::fs::write(&file, "considerably longer content").expect("rewrite");
        let second = FileVersion::current(&file).expect("present");

        assert_ne!(first, second);
    }

    /// The key must not depend on whether the file is there, or the two
    /// transitions this module exists to catch both report "never looked".
    #[test]
    fn an_observation_survives_the_file_appearing_and_vanishing() {
        let registry = ObservationRegistry::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("a.rs");

        registry.record(&observer("s"), &file);
        assert_eq!(registry.len(&observer("s")), 1);

        std::fs::write(&file, "x").expect("write");
        assert_ne!(
            registry.verdict(&observer("s"), &file),
            Verdict::Unobserved,
            "the observation was lost when the file appeared"
        );

        registry.record(&observer("s"), &file);
        std::fs::remove_file(&file).expect("remove");
        assert_ne!(
            registry.verdict(&observer("s"), &file),
            Verdict::Unobserved,
            "the observation was lost when the file vanished"
        );
        assert_eq!(
            registry.len(&observer("s")),
            1,
            "one file must not occupy two keys"
        );
    }

    #[test]
    fn a_missing_file_has_no_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(FileVersion::current(&dir.path().join("nope")), None);
    }
}
