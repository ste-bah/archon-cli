// TASK-JEVAL-012 — JepaEvalRunStore
//
// Persistent run state that makes evals observable, resumable, and
// concurrency-safe (PRD-006C §6.2; REQ-JEVAL-20..23).
//
// All types are flat under crate::jepa::* per DEC-JEVAL-11.
//
// Note: PathBuf, Result, Utc, DateTime, Serialize, Deserialize, and Write
// are already in scope from 00_config_metadata.rs (included earlier in the
// flat include!() module).
//
// On-disk layout (all under run_dir, typically
//   ~/.archon/world-model/jepa/eval-runs/):
//
//   <run-id>.json         — atomic run record (temp+rename)
//   <candidate-id>.lock   — O_EXCL per-candidate lock (JSON content)
//   <run-id>.cancel       — empty sentinel; presence => cancel requested
//   <run-id>.log          — stdout/stderr of background worker (T013)

// ---------------------------------------------------------------------------
// Internal lock-file record
// ---------------------------------------------------------------------------

/// Content of the per-candidate lock file.
/// Written as JSON so it is human-readable for debugging.
#[derive(Debug, Serialize, Deserialize)]
struct CandidateLockRecord {
    pid: u32,
    run_id: String,
    host: String,
    acquired_at: chrono::DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// RAII lock guard
// ---------------------------------------------------------------------------

/// RAII guard: removes the lock file on Drop even on early-return / panic.
#[derive(Debug)]
pub struct CandidateLock {
    lock_path: PathBuf,
}

impl Drop for CandidateLock {
    fn drop(&mut self) {
        // Best-effort; ignore errors (file may already be gone on reclaim).
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Manages atomic run records, per-candidate locks, and cancel sentinels
/// under a single directory (typically ~/.archon/world-model/jepa/eval-runs/).
pub struct JepaEvalRunStore {
    run_dir: PathBuf,
}

impl JepaEvalRunStore {
    /// Create the store, creating `run_dir` if it does not exist.
    pub fn new(run_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&run_dir)?;
        Ok(Self { run_dir })
    }

    // -----------------------------------------------------------------------
    // Path helpers
    // -----------------------------------------------------------------------

    /// Path of the run record JSON file.
    pub fn run_path(&self, run_id: &str) -> PathBuf {
        self.run_dir.join(format!("{run_id}.json"))
    }

    /// Path of the per-candidate lock file.
    pub fn lock_path(&self, candidate_id: &str) -> PathBuf {
        self.run_dir.join(format!("{candidate_id}.lock"))
    }

    /// Path of the cancel sentinel file.
    pub fn cancel_path(&self, run_id: &str) -> PathBuf {
        self.run_dir.join(format!("{run_id}.cancel"))
    }

    /// Path of the background worker log file.
    pub fn log_path(&self, run_id: &str) -> PathBuf {
        self.run_dir.join(format!("{run_id}.log"))
    }

    // -----------------------------------------------------------------------
    // Run-id generation
    // -----------------------------------------------------------------------

    /// Generate a new unique run ID formatted as `"jeval-<uuid-v4>"`.
    pub fn generate_run_id() -> String {
        format!("jeval-{}", uuid::Uuid::new_v4())
    }

    // -----------------------------------------------------------------------
    // Atomic write / read / list
    // -----------------------------------------------------------------------

    /// Write `record` atomically via temp-file + rename.
    /// Guarantees no torn reads even if the process dies mid-write.
    pub fn write_run(&self, record: &crate::jepa::JepaEvalRunRecord) -> Result<()> {
        let path = self.run_path(&record.run_id);
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(record)?;
        std::fs::write(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Read and deserialize a run record by run-id.
    pub fn read_run(&self, run_id: &str) -> Result<crate::jepa::JepaEvalRunRecord> {
        let path = self.run_path(run_id);
        let json = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&json)?)
    }

    /// List up to `limit` run records, sorted newest-first by `started_at`.
    /// Non-JSON files and temporary `.json.tmp` files are silently skipped.
    pub fn list_runs(&self, limit: usize) -> Result<Vec<crate::jepa::JepaEvalRunRecord>> {
        let mut records = Vec::new();
        for entry in std::fs::read_dir(&self.run_dir)? {
            let entry = entry?;
            let path = entry.path();
            // Accept only *.json; skip *.json.tmp and other extensions.
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let stem = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                if stem.ends_with(".tmp") {
                    continue;
                }
                if let Ok(json) = std::fs::read_to_string(&path) {
                    if let Ok(r) =
                        serde_json::from_str::<crate::jepa::JepaEvalRunRecord>(&json)
                    {
                        records.push(r);
                    }
                }
            }
        }
        records.sort_by_key(|r| std::cmp::Reverse(r.started_at));
        records.truncate(limit);
        Ok(records)
    }

    // -----------------------------------------------------------------------
    // Cancel sentinel
    // -----------------------------------------------------------------------

    /// Create an empty cancel sentinel file for `run_id`.
    /// The background worker polls this via `cancel_sentinel_exists`.
    pub fn write_cancel_sentinel(&self, run_id: &str) -> Result<()> {
        std::fs::write(self.cancel_path(run_id), b"")?;
        Ok(())
    }

    /// Return `true` if the cancel sentinel file exists for `run_id`.
    pub fn cancel_sentinel_exists(&self, run_id: &str) -> bool {
        self.cancel_path(run_id).exists()
    }

    // -----------------------------------------------------------------------
    // Per-candidate O_EXCL lock
    // -----------------------------------------------------------------------

    /// Acquire an exclusive per-candidate lock using O_CREAT | O_EXCL.
    ///
    /// Returns a `CandidateLock` RAII guard that removes the lock file on Drop.
    ///
    /// # Errors
    /// Returns `Err` containing "already in progress" and the existing run-id
    /// if the lock file already exists (ERR-JEVAL-03).
    pub fn acquire_candidate_lock(
        &self,
        candidate_id: &str,
        run_id: &str,
    ) -> Result<CandidateLock> {
        let lock_path = self.lock_path(candidate_id);
        let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
        let record = CandidateLockRecord {
            pid: std::process::id(),
            run_id: run_id.to_string(),
            host,
            acquired_at: Utc::now(),
        };
        let json = serde_json::to_string(&record)?;

        // O_CREAT | O_EXCL — fails atomically if the file already exists.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut f) => {
                f.write_all(json.as_bytes())?;
                Ok(CandidateLock { lock_path })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Read the existing lock for a helpful error message.
                let existing_json =
                    std::fs::read_to_string(&lock_path).unwrap_or_default();
                let existing: CandidateLockRecord =
                    serde_json::from_str(&existing_json).unwrap_or(CandidateLockRecord {
                        pid: 0,
                        run_id: "unknown".into(),
                        host: "unknown".into(),
                        acquired_at: Utc::now(),
                    });
                Err(anyhow::anyhow!(
                    "Eval run {} is already in progress for candidate {} \
                     (pid: {}, host: {}).\n\
                     Use: archon world eval-jepa-cancel {} \
                     to cancel the existing run.",
                    existing.run_id,
                    candidate_id,
                    existing.pid,
                    existing.host,
                    existing.run_id,
                ))
            }
            Err(e) => Err(anyhow::anyhow!("Failed to acquire candidate lock: {e}")),
        }
    }

    // -----------------------------------------------------------------------
    // Stale-lock reclamation
    // -----------------------------------------------------------------------

    /// Attempt to reclaim a stale lock file for `candidate_id`.
    ///
    /// Staleness algorithm:
    /// - If the holding pid is dead (`kill(pid, 0)` -> ESRCH) -> stale.
    /// - `budget_ms == 0` (unlimited run): stale when the heartbeat age
    ///   (`Utc::now() - run.updated_at`) exceeds `stale_heartbeat_ms`.
    /// - `budget_ms > 0` (bounded run): stale when
    ///   `elapsed > budget_ms + 30_000` (30 s grace).
    ///
    /// When reclaimed the on-disk run record's status is set to `Stale`.
    ///
    /// Returns `true` if the lock was stale and has been removed.
    pub fn reclaim_stale_lock_if_applicable(
        &self,
        candidate_id: &str,
        budget_ms: u64,
        stale_heartbeat_ms: u64,
    ) -> Result<bool> {
        let lock_path = self.lock_path(candidate_id);
        if !lock_path.exists() {
            return Ok(false);
        }
        let json = std::fs::read_to_string(&lock_path)?;
        let lock: CandidateLockRecord = serde_json::from_str(&json)?;

        let pid_alive = is_pid_alive(lock.pid);

        let is_stale = if !pid_alive {
            true
        } else if budget_ms > 0 {
            // Bounded budget: elapsed > budget + 30 s grace.
            const GRACE_MS: u64 = 30_000;
            let elapsed_ms = (Utc::now() - lock.acquired_at)
                .num_milliseconds()
                .max(0) as u64;
            elapsed_ms > budget_ms + GRACE_MS
        } else {
            // Unlimited budget: use heartbeat age on the run record.
            match self.read_run(&lock.run_id) {
                Ok(run_record) => {
                    let heartbeat_age_ms = (Utc::now() - run_record.updated_at)
                        .num_milliseconds()
                        .max(0) as u64;
                    heartbeat_age_ms > stale_heartbeat_ms
                }
                // Cannot read the run record — treat as stale.
                Err(_) => true,
            }
        };

        if is_stale {
            // Mark the run record as Stale (best-effort; don't abort reclaim on failure).
            if let Ok(mut run_record) = self.read_run(&lock.run_id) {
                run_record.status = crate::jepa::EvalRunStatus::Stale;
                run_record.updated_at = Utc::now();
                let _ = self.write_run(&run_record);
            }
            std::fs::remove_file(&lock_path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // -----------------------------------------------------------------------
    // Background worker (POSIX double-fork + setsid + exec)
    // -----------------------------------------------------------------------

    /// Spawn the current executable as a background worker using the POSIX
    /// double-fork + setsid pattern so the grandchild is fully detached from
    /// the controlling terminal.
    ///
    /// EXPERIMENTAL: The current implementation re-execs with
    /// `--__bg-worker --run-id <id>` flags that are NOT YET wired into the
    /// CLI dispatch — the grandchild will fail immediately on arg parsing.
    /// Callers should currently NOT invoke this method; use foreground eval
    /// instead. Wiring the dispatch entry point is deferred to a future task.
    ///
    /// The grandchild's stdout/stderr are redirected to `<run_id>.log` in the
    /// store's run directory; stdin is redirected to `/dev/null`.
    ///
    /// The grandchild receives `--__bg-worker --run-id <run_id>` appended to
    /// the current process's argument list, plus any `extra_args`.
    ///
    /// # Errors
    /// Always returns `Err` on Windows (ERR-JEVAL-04).
    pub fn spawn_background_worker(&self, run_id: &str, extra_args: &[&str]) -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            let _ = (run_id, extra_args);
            return Err(anyhow::anyhow!(
                "--background is not supported on Windows (ERR-JEVAL-04); \
                 run in the foreground or use eval-jepa-status from another shell."
            ));
        }

        #[cfg(not(target_os = "windows"))]
        {
            use std::ffi::CString;

            let log_path = self.log_path(run_id);
            let current_exe = std::env::current_exe()?;

            // Build args: existing argv + bg-worker flags + extra_args.
            let mut args: Vec<String> = std::env::args().collect();
            args.push("--__bg-worker".into());
            args.push("--run-id".into());
            args.push(run_id.to_string());
            for &a in extra_args {
                args.push(a.to_string());
            }

            // CStrings must outlive the unsafe block that uses their pointers.
            let exe_c = CString::new(
                current_exe
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("non-UTF-8 executable path"))?,
            )?;
            let c_args: Vec<CString> = args
                .iter()
                .map(|a| CString::new(a.as_str()))
                .collect::<std::result::Result<_, _>>()?;
            let mut c_ptrs: Vec<*const libc::c_char> =
                c_args.iter().map(|a| a.as_ptr()).collect();
            c_ptrs.push(std::ptr::null());

            let log_c = CString::new(
                log_path
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("non-UTF-8 log path"))?,
            )?;

            // SAFETY: standard double-fork + setsid daemonisation pattern.
            // After the first fork the child calls setsid() then forks again;
            // the grandchild execs the worker.  The first child exits
            // immediately so the grandchild is re-parented to init (PID 1) and
            // cannot become a zombie.  The parent waits for the first child to
            // prevent a zombie before returning.
            unsafe {
                match libc::fork() {
                    -1 => {
                        return Err(anyhow::anyhow!("fork() failed (first fork)"));
                    }
                    0 => {
                        // ---- First child ----
                        libc::setsid();
                        match libc::fork() {
                            0 => {
                                // ---- Grandchild: redirect stdio and exec ----

                                // stdin -> /dev/null
                                let devnull = libc::open(
                                    b"/dev/null\0".as_ptr() as *const libc::c_char,
                                    libc::O_RDONLY,
                                );
                                if devnull >= 0 {
                                    libc::dup2(devnull, 0);
                                    libc::close(devnull);
                                }

                                // stdout + stderr -> log file
                                let logfd = libc::open(
                                    log_c.as_ptr(),
                                    libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
                                    0o644_u32,
                                );
                                if logfd >= 0 {
                                    libc::dup2(logfd, 1);
                                    libc::dup2(logfd, 2);
                                    libc::close(logfd);
                                }

                                libc::execv(exe_c.as_ptr(), c_ptrs.as_ptr());
                                // execv only returns on failure.
                                libc::_exit(1);
                            }
                            _ => {
                                // First child exits immediately so grandchild is
                                // reparented to init.
                                libc::_exit(0);
                            }
                        }
                    }
                    _ => {
                        // ---- Parent: reap first child to avoid zombie ----
                        let mut status: libc::c_int = 0;
                        libc::waitpid(-1, &mut status, 0);
                    }
                }
            }

            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// PID liveness check
// ---------------------------------------------------------------------------

/// Return `true` if the process with `pid` is alive (POSIX: `kill(pid, 0)`).
/// Always returns `false` on Windows (conservative: treat unknown as dead).
#[cfg(not(target_os = "windows"))]
fn is_pid_alive(pid: u32) -> bool {
    // SAFETY: kill(pid, 0) sends no signal; it only checks process existence.
    // Returns 0 if the process exists and we have permission, -1 otherwise.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(target_os = "windows")]
fn is_pid_alive(_pid: u32) -> bool {
    false
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

