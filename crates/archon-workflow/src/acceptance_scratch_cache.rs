//! A persistent compiled-artifact cache for native observations (Batch J2).
//!
//! Without one, every observation builds the target from nothing in its own
//! `observation-<uuid>/target`: about half an hour for a large workspace on
//! a loaded host, for the acceptance stage's own round and for every commit
//! the regression search probes. With [`ScratchPolicy::build_cache`] set
//! (the live stage sets it per run), an observation:
//!
//! - holds the cache's exclusive lock for its whole life, so observations
//!   sharing a cache run one at a time;
//! - is laid out at ONE fixed path, `<cache>/scratch`, rebuilt from nothing
//!   every time exactly as before (a fresh worktree at the commit, a fresh
//!   project copy with its inputs, fresh HOME, TMPDIR and CARGO_HOME), so
//!   every absolute path Cargo fingerprints is the same from one
//!   observation to the next. Only the compiled artifacts are shared;
//! - builds into `<cache>/target` (CARGO_TARGET_DIR and the cwd `target`
//!   links), which survives teardown;
//! - gives every tracked file the modification time recorded for its exact
//!   bytes at that path (`<cache>/mtimes.json`); bytes not recorded there
//!   get the time of this prepare. Cargo judges a local crate fresh when no
//!   source is newer than its last build, so an unchanged file keeps an old
//!   time and a changed one is newer than every build that saw other bytes.
//!
//! Anything that could let a build see bytes other than those its recorded
//! time stands for makes the next observation build cold instead, in a new
//! generation of slot and target: a slot a previous holder never tore down,
//! a tracked file whose bytes or time differ at teardown from those given
//! (a check rewrote or touched it), a record that cannot be written, or a
//! target over half the scratch limit.
use super::*;
use std::time::SystemTime;

const RECORD: &str = "mtimes.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    secs: u64,
    nanos: u32,
}

impl Stamp {
    fn of(time: SystemTime) -> Self {
        let since = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            secs: since.as_secs(),
            nanos: since.subsec_nanos(),
        }
    }

    fn time(self) -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::new(self.secs, self.nanos)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Seen {
    digest: String,
    stamp: Stamp,
}

/// The held cache: dropping it releases the lock.
pub(super) struct Lease {
    dir: PathBuf,
    _lock: std::fs::File,
    /// The cache's generation: its slot and target are `scratch-<n>` and
    /// `target-<n>`. Forgetting every build moves to the next one, so a
    /// process still writing to the last one (a check orphaned by a killed
    /// guardian keeps its environment's absolute paths) never shares a
    /// directory with a later build.
    generation: u64,
    /// What each tracked file (`repo/<name>`, `project/<name>`) was given.
    given: BTreeMap<String, Seen>,
}

fn io_error(path: &Path, error: std::io::Error) -> WorkflowError {
    WorkflowError::io(path, error)
}

/// A cache no observation has used for this long, and none holds, is
/// emptied by the next observation of a sibling cache: a finished run's
/// cache does not outlive it by more than this.
const UNUSED_CACHE_EXPIRY: std::time::Duration = std::time::Duration::from_secs(7 * 86_400);

#[cfg(unix)]
fn try_lock(file: &std::fs::File, path: &Path) -> WorkflowResult<bool> {
    use std::os::fd::AsRawFd;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
        return Ok(false);
    }
    Err(io_error(path, error))
}

fn open_lock(path: &Path) -> WorkflowResult<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|e| io_error(path, e))
}

impl Lease {
    /// Lock the cache at `dir`, waiting (within the phase deadline) for any
    /// other holder. A slot left behind by one that never tore down, or a
    /// target over half of `limit` bytes (each check audits the scratch and
    /// the target together against it), moves the cache to its next
    /// generation: cold, never stale.
    pub(super) fn acquire(dir: &Path, limit: u64) -> WorkflowResult<Self> {
        std::fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
        let dir = dir.canonicalize().map_err(|e| io_error(dir, e))?;
        let path = dir.join("lock");
        let lock = open_lock(&path)?;
        #[cfg(unix)]
        while !try_lock(&lock, &path)? {
            control::check()?;
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        #[cfg(not(unix))]
        return Err(invalid("a shared native build cache requires a Unix host"));
        // Its last use, for the expiry of unused caches.
        let _ = lock.set_modified(SystemTime::now());
        let generation = std::fs::read_to_string(dir.join("generation"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(0);
        let mut lease = Self {
            dir,
            _lock: lock,
            generation,
            given: BTreeMap::new(),
        };
        let stale = !lease.leftovers("scratch-").is_empty();
        // Half the scratch limit, so this observation has room to build.
        let oversized = super::process::scratch_size(&lease.target()).is_ok_and(|n| n > limit / 2);
        if stale || oversized {
            lease.poison()?;
        }
        for old in lease.leftovers("target-") {
            let _ = io::remove_owned_tree(&old);
        }
        lease.prune_siblings();
        std::fs::create_dir_all(lease.target()).map_err(|e| io_error(&lease.target(), e))?;
        Ok(lease)
    }

    /// The path every observation of this generation is laid out at.
    pub(super) fn slot(&self) -> PathBuf {
        self.dir.join(format!("scratch-{}", self.generation))
    }

    pub(super) fn target(&self) -> PathBuf {
        self.dir.join(format!("target-{}", self.generation))
    }

    /// Entries named `<prefix><n>`: every slot (a holder that tore down
    /// leaves none), or the targets of forgotten generations.
    pub(super) fn leftovers(&self, prefix: &str) -> Vec<PathBuf> {
        let current = self.target();
        (std::fs::read_dir(&self.dir).into_iter().flatten().flatten())
            .map(|entry| entry.path())
            .filter(|path| {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                name.starts_with(prefix) && *path != current
            })
            .collect()
    }

    /// Forget every build: the record goes first, then the cache moves to
    /// its next generation and the old target is removed if it can be.
    fn poison(&mut self) -> WorkflowResult<()> {
        io::remove_owned_tree(&self.dir.join(RECORD))?;
        let old = self.target();
        self.generation += 1;
        let path = self.dir.join("generation");
        let staging = self.dir.join("generation.tmp");
        std::fs::write(&staging, self.generation.to_string()).map_err(|e| io_error(&staging, e))?;
        std::fs::rename(&staging, &path).map_err(|e| io_error(&path, e))?;
        let _ = io::remove_owned_tree(&old);
        Ok(())
    }

    /// Empty sibling caches no observation holds or has used for
    /// [`UNUSED_CACHE_EXPIRY`] and that left no slot behind (their lock
    /// file stays).
    fn prune_siblings(&self) {
        let Some(parent) = self.dir.parent() else {
            return;
        };
        for entry in std::fs::read_dir(parent).into_iter().flatten().flatten() {
            let sibling = entry.path();
            let path = sibling.join("lock");
            let expired = (path.symlink_metadata().and_then(|m| m.modified()).ok())
                .and_then(|used| used.elapsed().ok())
                .is_some_and(|age| age > UNUSED_CACHE_EXPIRY);
            if sibling == self.dir || !expired {
                continue;
            }
            let Ok(lock) = open_lock(&path) else {
                continue;
            };
            #[cfg(unix)]
            if try_lock(&lock, &path).ok() != Some(true) {
                continue;
            }
            let slots = std::fs::read_dir(&sibling).into_iter().flatten().flatten();
            if slots
                .into_iter()
                .any(|e| e.file_name().to_string_lossy().starts_with("scratch-"))
            {
                continue;
            }
            // Everything but the lock, which another opener may be
            // waiting on: its inode must stay the one they lock.
            for item in std::fs::read_dir(&sibling).into_iter().flatten().flatten() {
                if item.file_name() != "lock" {
                    let _ = io::remove_owned_tree(&item.path());
                }
            }
        }
    }

    fn record(&self) -> BTreeMap<String, Seen> {
        std::fs::read(self.dir.join(RECORD))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Give each tracked file under `roots` (label, root) its recorded time,
    /// or now when its bytes are not the ones recorded, and save the record
    /// -- this commit's files only -- before any build can run. A path not
    /// in this commit is forgotten: should it come back, it comes back new.
    pub(super) fn stabilize(
        &mut self,
        roots: &[(&str, &Path)],
        names: &[&str],
    ) -> WorkflowResult<()> {
        let recorded = self.record();
        let mut record = BTreeMap::new();
        let now = Stamp::of(SystemTime::now());
        for (label, root) in roots {
            for name in names {
                control::check()?;
                let path = root.join(name);
                match path.symlink_metadata() {
                    Ok(meta) if meta.is_file() => {}
                    _ => continue,
                }
                let key = format!("{label}/{name}");
                let digest = blake3::hash(&io::read(&path)?).to_hex().to_string();
                let stamp = match recorded.get(&key) {
                    Some(seen) if seen.digest == digest => seen.stamp,
                    _ => now,
                };
                let seen = Seen {
                    digest,
                    stamp: set_mtime(&path, stamp)?,
                };
                record.insert(key.clone(), seen.clone());
                self.given.insert(key, seen);
            }
        }
        let path = self.dir.join(RECORD);
        let staging = self.dir.join(format!("{RECORD}.tmp"));
        let saved = serde_json::to_vec(&record)
            .map_err(WorkflowError::from)
            .and_then(|bytes| std::fs::write(&staging, bytes).map_err(|e| io_error(&staging, e)))
            .and_then(|()| std::fs::rename(&staging, &path).map_err(|e| io_error(&path, e)));
        if let Err(error) = saved {
            self.poison()?;
            return Err(error);
        }
        Ok(())
    }

    /// Before teardown: a tracked file no longer holding the bytes and the
    /// time it was given was rewritten or touched by a check, so a build may
    /// have seen other bytes than its time stands for. Forget every build.
    pub(super) fn audit(&mut self, roots: &[(&str, &Path)]) -> WorkflowResult<()> {
        let intact = |path: &Path, seen: &Seen| {
            path.symlink_metadata()
                .is_ok_and(|m| m.is_file() && m.modified().ok().map(Stamp::of) == Some(seen.stamp))
                && io::read(path)
                    .is_ok_and(|bytes| blake3::hash(&bytes).to_hex().as_str() == seen.digest)
        };
        let touched = roots.iter().any(|(label, root)| {
            self.given.iter().any(|(key, seen)| {
                key.strip_prefix(label)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .is_some_and(|name| !intact(&root.join(name), seen))
            })
        });
        if touched { self.poison() } else { Ok(()) }
    }
}

/// Set `path`'s modification time without following a link; returns the
/// time the filesystem actually holds (its precision may be coarser).
fn set_mtime(path: &Path, stamp: Stamp) -> WorkflowResult<Stamp> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| io_error(path, e))?;
    file.set_modified(stamp.time())
        .map_err(|e| io_error(path, e))?;
    let held = file
        .metadata()
        .and_then(|m| m.modified())
        .map_err(|e| io_error(path, e))?;
    Ok(Stamp::of(held))
}
