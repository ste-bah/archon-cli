use super::*;

impl ResultStore {
    pub(crate) fn new(dir: PathBuf, redactor: Redactor) -> Self {
        Self {
            dir,
            redactor: Arc::new(redactor),
            oracle: None,
        }
    }

    /// This store, refusing to keep a result whose masked copy would read
    /// as a different verdict than its raw capture (Issue 544).
    pub(crate) fn with_oracle(mut self, oracle: VerdictOracle) -> Self {
        self.oracle = Some(oracle);
        self
    }

    pub(crate) fn path(&self, key: &str) -> Option<PathBuf> {
        (key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| self.dir.join(format!("{key}.json")))
    }

    pub(crate) fn evidence_ref(&self, key: &str) -> Option<String> {
        self.path(key).map(|path| path.display().to_string())
    }

    /// The verdict saved under `key`; incompatible evidence is a miss and is
    /// left in place for investigation. Schema 3 is safe only at Scratch,
    /// whose judging context supplies its fixed tree layout.
    pub(crate) fn load(&self, key: &str, allow_schema3: bool) -> Option<CheckEvidence> {
        let path = self.path(key)?;
        let saved = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok());
        match saved {
            Some(saved)
                if (saved.schema == SCHEMA
                    || (!allow_schema3 && saved.schema == 4)
                    || (allow_schema3 && saved.schema == 3))
                    && saved.key == key
                    && saved.result.operational_error.is_none()
                    && saved.result.classification.is_some()
                    && !super::verdict::may_be_host_failure(&saved.result) =>
            {
                Some(CheckEvidence {
                    result: saved.result,
                    copy_layout: saved.copy_layout,
                })
            }
            _ => None,
        }
    }

    /// Save `result` under `key`, atomically; false when it was not saved.
    #[cfg(test)]
    pub(crate) fn save(&self, key: &str, result: &CheckResult) -> bool {
        self.save_evidence(key, result, None, None)
    }

    /// Save `result` under `key`. Issue 544: `predicate` (a mutation run's
    /// own reuse rule) is also judged on the exact bytes to be persisted,
    /// against the raw capture: a stored copy that would decide differently
    /// is not kept.
    pub(crate) fn save_evidence(
        &self,
        key: &str,
        result: &CheckResult,
        copy_layout: Option<CopyLayout>,
        predicate: Option<&SavePredicate>,
    ) -> bool {
        let Some(path) = self.path(key) else {
            return false;
        };
        // Issue 328: nor a run that failed for its host (a program that
        // could not start, a tree that did not build): no verdict.
        if result.operational_error.is_some() || super::verdict::may_be_host_failure(result) {
            return false;
        }
        let original = result;
        let mut result = result.clone();
        // Native command results are already classified at capture. Declarative
        // results have no command and cannot be a script crash.
        result.classify_raw("");
        if let Some(classification) = &mut result.classification
            && let CheckRunClass::ScriptDefect(defect) = &mut classification.crash
        {
            defect.signal = "see the fenced stderr below".into();
            defect.rule =
                String::from_utf8_lossy(&self.redactor.redact_bytes(defect.rule.as_bytes()))
                    .into_owned();
        }
        result.stdout = self.redactor.redact_bytes(&result.stdout);
        result.stderr = self.redactor.redact_bytes(&result.stderr);
        // Issue 544: a reused verdict re-reads the stored (masked) text, so a
        // result is kept only when that text reads as the raw capture did:
        // the host's mutation protocol lines unchanged, and the same text
        // verdicts (no-verdict cause, setup path) at this site. Otherwise a
        // retry runs the check again and decides from its raw capture.
        if result.stdout != original.verdict_stdout() || result.stderr != original.verdict_stderr()
        {
            let raw = original.for_verdict();
            let mut masked = result.clone();
            masked.verdict_streams = Default::default();
            let protocol = |result: &CheckResult| -> Vec<String> {
                [&result.stdout, &result.stderr]
                    .iter()
                    .flat_map(|stream| {
                        String::from_utf8_lossy(stream)
                            .lines()
                            .filter(|line| line.starts_with("archon-mutation-"))
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .collect()
            };
            // The persisted candidate seen with the raw capture beside it, so
            // the predicate compares raw against the final masked bytes.
            let mut candidate = result.clone();
            candidate.verdict_streams = original.verdict_streams.clone();
            if protocol(&raw) != protocol(&masked)
                || self
                    .oracle
                    .as_ref()
                    .is_some_and(|same| !same(&raw, &masked))
                || predicate.is_some_and(|keep| !keep(&candidate))
            {
                return false;
            }
        }
        let saved = Saved {
            schema: SCHEMA,
            key: key.to_string(),
            result,
            copy_layout,
        };
        let path = path.with_extension("provisional");
        let staging = self.dir.join(format!(".{key}.{}.tmp", std::process::id()));
        let provisional = path.with_extension("provisional");
        let same_result = std::fs::read(&provisional)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|previous| {
                previous["key"] == key
                    && serde_json::to_value(&saved.result)
                        .is_ok_and(|result| previous["result"] == result)
            });
        let written = std::fs::create_dir_all(&self.dir).is_ok()
            && serde_json::to_vec(&saved).is_ok_and(|bytes| write_owner_only(&staging, &bytes))
            && (same_result || self.preserve_replaced(&provisional))
            && std::fs::rename(&staging, &provisional).is_ok();
        if !written {
            let _ = std::fs::remove_file(&staging);
        }
        written
    }

    /// The mutation nonce saved under `key` (see `mutation_markers`).
    pub(super) fn load_nonce(&self, key: &str) -> Option<String> {
        let path = self.path(key)?.with_extension("nonce");
        Some(std::fs::read_to_string(path).ok()?.trim().to_string())
    }

    pub(super) fn save_nonce(&self, key: &str, nonce: &str) {
        if let Some(path) = self.path(key) {
            let path = path.with_extension("nonce");
            let staging = path.with_extension(format!("nonce.{}.tmp", std::process::id()));
            let _ = std::fs::create_dir_all(&self.dir)
                .and_then(|()| std::fs::write(&staging, nonce))
                .and_then(|()| std::fs::rename(&staging, &path));
            let _ = std::fs::remove_file(&staging);
        }
    }

    pub(crate) fn remove(&self, key: &str) {
        if let Some(path) = self.path(key) {
            let _ = std::fs::remove_file(path.with_extension("provisional"));
        }
    }

    /// Keep prior evidence before promotion atomically replaces the active key.
    pub(super) fn preserve_replaced(&self, path: &std::path::Path) -> bool {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "could not read prior acceptance evidence for archival"
                );
                return false;
            }
        };
        let suffix = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .map(|value| {
                if value["schema"] == 3 {
                    "schema3"
                } else if value["schema"] == 4 && value["copy_layout"].is_null() {
                    "schema4-layoutless"
                } else {
                    "previous"
                }
            })
            .unwrap_or("previous");
        // Each distinct prior result adds one archive file; cleanup is intentionally absent so evidence is kept.
        for index in 0.. {
            let ext = if index == 0 {
                suffix.to_string()
            } else {
                format!("{suffix}.{index}")
            };
            let archive = path.with_extension(ext);
            match std::fs::hard_link(path, &archive) {
                Ok(()) => return true,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if std::fs::read(&archive).is_ok_and(|archived| archived == bytes) {
                        return true;
                    }
                }
                Err(_) if write_archive(&archive, &bytes) => return true,
                Err(_) => return false,
            }
        }
        false
    }
}
