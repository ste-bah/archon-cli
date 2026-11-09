//! Optional read-only requirement proof diagnostics.
use archon_workflow::baseline_results::results_digest;
use archon_workflow::breaker_verdict::{ProofVerdicts, Relevance, VERDICT_SCHEMA};
use archon_workflow::task_set_contract::AcceptanceLock;
use archon_workflow::v2::requirement_classes::RequirementClassification;
use std::collections::BTreeMap;
use std::path::Path;
fn label(v: &impl serde::Serialize) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}
pub(super) fn render(
    dir: &Path,
    lock: &AcceptanceLock,
    classification: &RequirementClassification,
) -> (String, bool) {
    let result = (|| -> anyhow::Result<Option<(String, bool)>> {
        let bytes = match std::fs::read(
            dir.join(lock.digest.get(..16).unwrap_or(""))
                .join("verdicts.json"),
        ) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let record: ProofVerdicts = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            record.schema == VERDICT_SCHEMA,
            "incompatible verdict schema"
        );
        let stale = record.acceptance_digest != lock.digest
            || record.baseline_results_digest != results_digest(&lock.baseline_results)
            || classification
                .base_snapshot
                .as_ref()
                .is_none_or(|s| s.digest != record.snapshot_digest || !s.valid_digest());
        let mut counts = BTreeMap::new();
        for p in &record.entries {
            *counts.entry(label(&p.verdict)).or_insert(0usize) += 1;
        }
        let counts = counts
            .into_iter()
            .map(|(k, n)| format!("{k}={n}"))
            .collect::<Vec<_>>()
            .join(" ");
        let unjudged = record
            .entries
            .iter()
            .flat_map(|p| p.survivors.iter())
            .filter(|s| s.equivalence_unjudged)
            .count();
        let mut out = format!(
            "breaker: rounds={} runs_executed={} runs_reused={} equivalence_distinct={} equivalence_unjudged={unjudged} {counts} stale={stale}\n",
            record.rounds,
            record.runs_executed,
            record.runs_reused,
            record.judges.equivalence_distinct
        );
        out.push_str(
            "breaker: each judgment uses at most 8 completions, subject to configured caps\n",
        );
        for p in &record.entries {
            let relevant = p
                .relevance
                .iter()
                .filter(|r| r.relevance == Relevance::Relevant)
                .map(|r| r.mutant_id.clone())
                .collect::<Vec<_>>();
            let survivors = p
                .survivors
                .iter()
                .map(|s| s.mutant_id.clone())
                .collect::<Vec<_>>();
            let joined = |v: Vec<String>| {
                if v.is_empty() {
                    "-".into()
                } else {
                    v.join(",")
                }
            };
            let evidence = p
                .survivors
                .first()
                .and_then(|s| s.patch.rsplit_once('/'))
                .map(|(p, _)| {
                    dir.join(archon_workflow::base_snapshot::key_path(p))
                        .display()
                        .to_string()
                })
                .or_else(|| {
                    std::fs::read_dir(dir.join("evidence"))
                        .ok()?
                        .filter_map(Result::ok)
                        .find_map(|entry| {
                            p.killed_by
                                .keys()
                                .any(|id| entry.path().join(format!("{id}.patch")).is_file())
                                .then(|| entry.path().display().to_string())
                        })
                })
                .unwrap_or_else(|| "-".into());
            out.push_str(&format!("breaker {} {}: {} attempts={} relevant={} survivors={} owed={} evidence={evidence}\n",p.task_id,p.id,label(&p.verdict),p.attempts.len(),joined(relevant),joined(survivors),p.owed.as_ref().map(|o|o.sup_id.as_str()).unwrap_or("-")));
        }
        Ok(Some((out, !stale)))
    })();
    match result {
        Ok(Some(v)) => v,
        Ok(None) => (String::new(), false),
        Err(e) => (format!("breaker: unreadable ({e:#})\n"), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_fresh_stale_and_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let mut lock: AcceptanceLock = serde_json::from_slice(include_bytes!(
            "../../crates/archon-workflow/tests/fixtures/classification/acceptance.lock"
        ))
        .unwrap();
        lock.digest = "a".repeat(64);
        let mut snapshot = archon_workflow::base_snapshot::BaseSnapshot {
            landing_verified: Default::default(),
            commit: "base".into(),
            entries: BTreeMap::new(),
            digest: String::new(),
        };
        snapshot.digest = snapshot.recomputed_digest();
        let classification:RequirementClassification=serde_json::from_value(serde_json::json!({"schema_version":2,"baseline_commit":"base","acceptance_digest":lock.digest,"baseline_results_digest":results_digest(&lock.baseline_results),"error":null,"entries":[],"base_snapshot":snapshot})).unwrap();
        assert_eq!(
            render(dir.path(), &lock, &classification),
            (String::new(), false)
        );
        let path = dir.path().join(&lock.digest[..16]).join("verdicts.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut record = serde_json::json!({"schema":1,"base_commit":"base","acceptance_digest":lock.digest,"baseline_results_digest":results_digest(&lock.baseline_results),"snapshot_digest":snapshot.digest,"rounds":1,"runs_executed":5,"runs_reused":0,"judges":{"breaker":{"model":"local"},"relevance":{"model":"local"},"equivalence":[],"equivalence_distinct":false}});
        std::fs::write(&path, record.to_string()).unwrap();
        let (text, fresh) = render(dir.path(), &lock, &classification);
        assert!(fresh);
        assert!(text.contains("rounds=1 runs_executed=5"));
        assert!(text.contains("stale=false"));
        record["snapshot_digest"] = "changed".into();
        std::fs::write(&path, record.to_string()).unwrap();
        let (text, fresh) = render(dir.path(), &lock, &classification);
        assert!(!fresh);
        assert!(text.contains("stale=true"));
        std::fs::write(&path, "invalid").unwrap();
        let (text, fresh) = render(dir.path(), &lock, &classification);
        assert!(!fresh);
        assert!(text.starts_with("breaker: unreadable ("));
    }
}
