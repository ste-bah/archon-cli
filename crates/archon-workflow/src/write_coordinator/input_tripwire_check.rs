//! Compare and restore the armed inputs, recording detection before repair.
use super::*;

impl InputTripwire {
    /// Compare with now. `Some` when something other than the host changed
    /// an input since [`Self::arm`]; every such change is restored where it
    /// safely can be, and the violation is logged under the run.
    pub fn check(self, call: &str) -> Option<EnvironmentViolation> {
        self.check_inner(call, true, false, |_| Ok(()))
            .ok()
            .flatten()
    }

    pub(super) fn check_recording(
        self,
        call: &str,
        allow_restore: bool,
        record: impl FnMut(&EnvironmentViolation) -> crate::WorkflowResult<()>,
    ) -> crate::WorkflowResult<Option<EnvironmentViolation>> {
        self.check_inner(call, allow_restore, true, record)
    }

    fn check_inner(
        self,
        call: &str,
        allow_restore: bool,
        durable: bool,
        mut record: impl FnMut(&EnvironmentViolation) -> crate::WorkflowResult<()>,
    ) -> crate::WorkflowResult<Option<EnvironmentViolation>> {
        let _section = host_write_section();
        let detected_at = host_sequence();
        let (now_paths, _) = walk(&self.policy);
        let mut all: Vec<String> = self.files.keys().cloned().collect();
        all.extend(now_paths);
        all.sort();
        all.dedup();
        let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
        let backup_dir = self
            .run_root
            .join("write-coordination")
            .join("environment-violations")
            .join(format!("{}-{stamp}", sanitize(call)));
        let mut changed = Vec::new();
        for rel in all {
            let destination = self.policy.project.join(&rel);
            let before = match self.files.get(&rel) {
                Some(state) => state.clone(),
                // Past the recorded cap: never judged.
                None if !self.complete => continue,
                None => "absent".to_string(),
            };
            let (after, bytes) = state_of(&destination);
            if after == before {
                continue;
            }
            // A write-capable call's own delivery: its work, recorded so the
            // divergence repair knows who put that copy there.
            if self.exempt.iter().any(|root| destination.starts_with(root)) {
                let _ = delivered(&self.run_root, call, &rel, &after);
                continue;
            }
            // Another write call in flight owns this path: its work, judged
            // by its own tripwire, never restored from under it.
            if in_flight_owns(&destination) {
                continue;
            }
            let (host, reaches) = if allow_restore {
                host_writes_since(&destination, self.armed_at)
            } else {
                (Vec::new(), false)
            };
            if reaches && host.last() == Some(&after) {
                continue;
            }
            let mut change = ChangedInput {
                path: rel.clone(),
                before: before.clone(),
                after,
                restored: false,
                note: String::new(),
            };
            let mut detected = changed.clone();
            detected.push(change.clone());
            record(&EnvironmentViolation {
                call: call.into(),
                changed: detected,
                backup_dir: backup_dir.clone(),
                attributed: false,
            })?;
            if let Some(bytes) = &bytes {
                let backup = backup_dir.join(
                    crate::write_coordinator::project_inputs::external::stored(&rel),
                );
                let kept = backup
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&backup, bytes));
                if let Err(error) = kept {
                    change.note = format!("its changed copy could not be kept: {error}");
                    changed.push(change);
                    continue;
                }
            }
            change.note = if !allow_restore {
                "the original process's host-write history is unavailable; restore the recorded pre-call bytes before resuming".into()
            } else {
                match self.restore(&destination, &before, &host, reaches) {
                    Ok(()) => {
                        change.restored = true;
                        String::new()
                    }
                    Err(why) => why,
                }
            };
            changed.push(change);
        }
        if !changed.is_empty() {
            remember_violation(detected_at, &self.policy.project, call, &changed);
        }
        let own = changed.len();
        // Overlapping calls cannot be told apart: a change another call's
        // check found (and the host restored) inside this call's window fails
        // this call too, so the culprit never passes on a restored file.
        for (other, change) in recent_violations_since(self.armed_at, &self.policy.project, call) {
            if !changed.iter().any(|mine| mine.path == change.path) {
                changed.push(ChangedInput {
                    note: format!(
                        "changed during this call; found and handled when {other} was checked"
                    ),
                    ..change
                });
            }
        }
        if changed.is_empty() {
            return Ok(None);
        }
        let attributed = own > 0 && changed.len() == own && !self.window.overlapped();
        let violation = EnvironmentViolation {
            call: call.to_string(),
            changed,
            backup_dir,
            attributed,
        };
        // Owned comparisons log durably in their pending registration's
        // completion, before removing that registration.
        if !durable {
            let _ = log(&self.run_root, &violation);
        }
        Ok(Some(violation))
    }

    fn restore(
        &self,
        destination: &Path,
        before: &str,
        host: &[String],
        reaches: bool,
    ) -> Result<(), String> {
        if !host.is_empty() || !reaches {
            return Err(
                "the host itself wrote this file during the call; putting back the pre-call copy would undo that, so a person must look".into(),
            );
        }
        // Issue-226: an external file is restored within its own tree.
        let project = (self.policy.external.tree_of(destination)).unwrap_or(&self.policy.project);
        if before == "absent" {
            refuse_links(project, destination).map_err(|e| e.to_string())?;
            return remove_input(destination).map_err(|e| e.to_string());
        }
        let object = objects_dir(&self.run_root).join(before);
        let bytes = read_no_follow(&object)
            .ok()
            .filter(|bytes| blake3::hash(bytes).to_hex().to_string() == before)
            .ok_or_else(|| format!("no kept copy of its pre-call state ({})", short(before)))?;
        if std::fs::symlink_metadata(destination).is_ok_and(|m| m.is_dir()) {
            return Err("a directory now stands where the file was".into());
        }
        write_file(project, destination, &bytes)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
