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
    /// Build a token from what a world reports about a path (#201 Phase 1).
    ///
    /// The caller is whichever [`FileSystem`](crate::filesystem::FileSystem)
    /// holds the file, so the token describes the bytes a write will land on
    /// rather than a same-named file on the host.
    ///
    /// A world that cannot report a modification time degrades the token to
    /// length alone, which cannot see a modification that preserved the length.
    /// That is weaker than the host token, and deliberately not hidden: the
    /// alternative is refusing every write in such a world. A backend that can
    /// do better should override `FileSystem::version` with a content hash.
    #[must_use]
    pub fn from_parts(len: u64, modified_nanos: Option<u128>) -> Self {
        let modified = modified_nanos.map_or_else(|| "?".to_string(), |nanos| nanos.to_string());
        Self(format!("{len}:{modified}"))
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
    /// Record what a caller saw at `path`.
    ///
    /// Called after a tool has genuinely shown the agent those bytes. The
    /// caller supplies the observation because only it knows which world it
    /// read — under a sandbox the host's copy of a path is not the file being
    /// edited, and a token minted here would describe the wrong one (#201).
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
    ///
    /// `current` is this path's version *in the world the write will land in*,
    /// or `None` when nothing is there. The caller reads it rather than this
    /// module, for the reason given on [`record_as`](Self::record_as).
    #[must_use]
    pub fn verdict(
        &self,
        observer: &Observer,
        path: &Path,
        current: Option<FileVersion>,
    ) -> Verdict {
        let Some(observation) = self.observation(observer, path) else {
            return Verdict::Unobserved;
        };
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

    /// Drop what one agent observed, leaving every other agent alone.
    ///
    /// A session ends once per process; subagents end constantly inside one.
    /// `execute-plan` runs a fresh subagent per task and each gets its own
    /// [`Observer`], so without this the map keeps a record per agent for the
    /// life of the process — including for agents that finished hours ago and
    /// whose ids nothing will ever present again.
    ///
    /// This is deliberately not eviction. A bounded cache is the right shape
    /// for something advisory, and this is not advisory: dropping an entry
    /// turns [`Verdict::Fresh`] into [`Verdict::Unobserved`], and under the
    /// default `read_before_edit = "block"` that refuses a write the agent had
    /// every right to make. Which write got refused would depend on how many
    /// other files happened to be in the map, which is not a behaviour anyone
    /// can reason about. A lifecycle boundary is nameable instead: what gets
    /// dropped, and when, is the same every time and can be stated in one
    /// sentence — the caller states it.
    ///
    /// Hence also the caller: the subagent executor's unconditional
    /// `handle_inner_complete`, not the `SubagentStop` hook.
    /// `crates/archon-tools/src/board/leases.rs` documents why that hook is the
    /// wrong seam for anything that must always run — it fires from
    /// `on_visible_complete`, which the `AutoBackgrounded` arm skips, so the
    /// longest-lived agents (the ones holding the most observations) would be
    /// exactly the ones never released.
    pub fn forget_agent(&self, observer: &Observer) {
        if let Ok(mut seen) = self.seen.lock() {
            seen.remove(observer);
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
#[path = "file_observation_tests.rs"]
mod tests;
