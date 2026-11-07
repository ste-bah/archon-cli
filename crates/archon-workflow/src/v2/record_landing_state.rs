//! One atomic commit binds record bytes to the call's receipt and refusals.
use super::*;
use std::collections::BTreeSet;

#[derive(Serialize, Deserialize)]
struct SavedCall {
    records: BTreeMap<String, Value>,
    changed: BTreeSet<String>,
    // Canonical pre-call task/record digests; None means the subject was absent.
    pre_call: BTreeMap<String, Option<Vec<u8>>>,
    rejected: BTreeMap<String, Vec<String>>,
    repair_required: bool,
    frozen_tasks: Option<BTreeMap<String, String>>,
}
impl RecordLanding {
    /// `subjects_are_tasks` says whether each subject is a canonical task id (map calls) or an opaque call id (reduce calls); it decides whether the subject may stand in for a finding's task.
    pub fn open(
        root: PathBuf,
        identity: String,
        kind: RecordKind,
        subjects: Vec<String>,
        subjects_are_tasks: bool,
    ) -> WorkflowResult<Self> {
        Self::open_mode(root, identity, kind, subjects, subjects_are_tasks, false)
    }

    /// Read the committed call snapshot without creating, rewriting or removing files.
    pub fn open_read_only(
        root: PathBuf,
        identity: String,
        kind: RecordKind,
        subjects: Vec<String>,
        subjects_are_tasks: bool,
    ) -> WorkflowResult<Self> {
        Self::open_mode(root, identity, kind, subjects, subjects_are_tasks, true)
    }

    fn open_mode(
        root: PathBuf,
        identity: String,
        kind: RecordKind,
        subjects: Vec<String>,
        subjects_are_tasks: bool,
        read_only: bool,
    ) -> WorkflowResult<Self> {
        if !read_only {
            std::fs::create_dir_all(&root).map_err(invalid)?;
        }
        let path = root.join("contract.json");
        let expected = json!({"identity":identity,"kind":kind,"subjects":subjects});
        if path.exists() {
            let saved: Value = serde_json::from_slice(&std::fs::read(&path).map_err(invalid)?)?;
            if saved != expected {
                return Err(invalid("record landing identity changed"));
            }
        } else if read_only {
            return Err(invalid("missing record landing contract"));
        } else {
            write(&path, &expected)?;
        }
        let mut store = Self {
            read_snapshot: None,
            root,
            kind,
            subjects,
            subjects_are_tasks,
            lock: Mutex::new(()),
            frozen_tasks: Mutex::new(None),
            rejected: Mutex::new(BTreeMap::new()),
            landed_here: Mutex::new(Default::default()),
            repair_required: Mutex::new(false),
            pre_call_tasks: Mutex::new(Default::default()),
        };
        if read_only {
            store.read_snapshot = Some(if store.root.join("call-state.json").exists() {
                BTreeMap::new()
            } else {
                store.saved_records()?
            });
        }
        store.restore_call()?;
        Ok(store)
    }

    /// Whether this call must repair open findings, rather than author a first draft.
    pub fn require_new_landings(&self, required: bool) -> WorkflowResult<()> {
        self.ensure_writable()?;
        let _lock = self.lock.lock().map_err(invalid)?;
        *self.repair_required.lock().map_err(invalid)? = required;
        self.save_call()
    }

    pub(in crate::v2) fn saved_records(&self) -> WorkflowResult<BTreeMap<String, Value>> {
        let mut records = BTreeMap::new();
        for entry in std::fs::read_dir(&self.root).map_err(invalid)? {
            let path = entry.map_err(invalid)?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with("record-") && name.ends_with(".json") {
                records.insert(
                    name.to_owned(),
                    serde_json::from_slice(&std::fs::read(&path).map_err(invalid)?)?,
                );
            }
        }
        Ok(records)
    }
    pub(super) fn ensure_writable(&self) -> WorkflowResult<()> {
        if self.read_snapshot.is_some() {
            return Err(invalid("record landing store is read-only"));
        }
        Ok(())
    }
    pub(super) fn save_call(&self) -> WorkflowResult<()> {
        self.ensure_writable()?;
        write(
            &self.root.join("call-state.json"),
            &SavedCall {
                records: self.saved_records()?,
                changed: self.landed_here.lock().map_err(invalid)?.clone(),
                pre_call: self.pre_call_tasks.lock().map_err(invalid)?.clone(),
                rejected: self.rejected.lock().map_err(invalid)?.clone(),
                repair_required: *self.repair_required.lock().map_err(invalid)?,
                frozen_tasks: self.frozen_tasks.lock().map_err(invalid)?.clone(),
            },
        )
    }
    pub(super) fn restore_call(&mut self) -> WorkflowResult<()> {
        let path = self.root.join("call-state.json");
        if !path.exists() {
            return Ok(());
        }
        let saved: SavedCall = serde_json::from_slice(&std::fs::read(path).map_err(invalid)?)?;
        // Validate names before using the snapshot as paths.
        if saved.records.keys().any(|name| {
            !name.starts_with("record-")
                || !name.ends_with(".json")
                || std::path::Path::new(name).components().count() != 1
        }) {
            return Err(invalid("invalid saved record path"));
        }
        if self.read_snapshot.is_some() {
            self.read_snapshot = Some(saved.records);
        } else {
            // A process crash before the atomic state rename leaves the prior commit.
            // Repeating this recovery after another crash is safe.
            for name in self.saved_records()?.keys() {
                if !saved.records.contains_key(name) {
                    std::fs::remove_file(self.root.join(name)).map_err(invalid)?;
                }
            }
            for (name, record) in saved.records {
                write(&self.root.join(name), &record)?;
            }
        }
        *self.landed_here.lock().map_err(invalid)? = saved.changed;
        *self.pre_call_tasks.lock().map_err(invalid)? = saved.pre_call;
        *self.rejected.lock().map_err(invalid)? = saved.rejected;
        *self.repair_required.lock().map_err(invalid)? = saved.repair_required;
        *self.frozen_tasks.lock().map_err(invalid)? = saved.frozen_tasks;
        Ok(())
    }
}
