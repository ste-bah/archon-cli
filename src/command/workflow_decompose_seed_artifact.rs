//! Whole-artifact recovery keeps a malformed attempt from replacing judged work.
use super::*;
use archon_workflow::v2::record_landing::{RecordKind, RecordLanding};
use std::path::Path;

fn skeleton(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .is_some_and(|v| v["tasks"].is_array())
}
fn artifact_only(gate: &GateSeed) -> bool {
    !gate.findings.is_empty()
        && gate
            .findings
            .iter()
            .all(|f| f["remediation_scope"] == "candidate_artifact")
}

pub(super) fn derive(
    derived: &mut Derived,
    artifacts: BTreeMap<String, (u64, &WorkflowV2CallRecord)>,
    gates: &[Gate<'_>],
) {
    for (subject, (_, record)) in artifacts {
        let mut candidate = reply_content(record).unwrap_or_default();
        let mut candidate_call = record.call.id.clone();
        let mut gate = gates
            .iter()
            .rfind(|(_, _, stdin, _)| *stdin == candidate)
            .map(|(_, _, _, gate)| gate.clone());
        if subject == "skeleton"
            && (!skeleton(candidate) || gate.as_ref().is_some_and(artifact_only))
            && let Some((_, _, stdin, judged)) = gates.iter().rfind(|(_, command, stdin, gate)| {
                *command == "freeze-skeleton" && skeleton(stdin) && !artifact_only(gate)
            })
        {
            candidate = stdin;
            gate = Some(judged.clone());
            // A gate can hold host-composed bytes that no author returned verbatim.
            candidate_call = judged.call_id.clone();
        }
        derived.subjects.insert(
            subject,
            SubjectSeed::Artifact {
                candidate: candidate.to_owned(),
                candidate_call,
                gate,
            },
        );
    }
}

/// Call ordinals choose the work to recover. New stores name their author;
/// legacy stores can be attributed only when their contract was created inside
/// exactly that recorded call's interval. Within that call, the newest readable
/// store with landed records wins; ambiguous sessions keep judged bytes.
pub(crate) fn retain_skeleton(
    derived: &mut Derived,
    records: &[WorkflowV2CallRecord],
    v2_root: &Path,
) -> Result<()> {
    let Some(SubjectSeed::Artifact {
        candidate,
        candidate_call,
        ..
    }) = derived.subjects.get_mut("skeleton")
    else {
        return Ok(());
    };
    let latest = records
        .iter()
        .filter_map(|record| {
            let (subject, _, ordinal) = author_call(&record.call.id)?;
            (subject == "skeleton")
                .then(|| reply_content(record).map(|text| (ordinal, text)))
                .flatten()
        })
        .max_by_key(|(ordinal, _)| *ordinal);
    let ordered = ordered(records)?;
    if let Some((_, text)) = latest
        && skeleton(text)
        && !ordered
            .iter()
            .filter_map(|record| judged_gate(record))
            .filter(|(command, stdin, _)| *command == "freeze-skeleton" && *stdin == text)
            .next_back()
            .is_some_and(|(_, _, gate)| artifact_only(&gate))
    {
        return Ok(());
    }
    let root = v2_root.join("stage-records");
    let identities = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            tracing::warn!(store = %root.display(), %error, "skipping unreadable skeleton stores");
            return Ok(());
        }
    };
    let mut stores = Vec::new();
    for identity in identities {
        let identity = match identity {
            Ok(identity) => identity,
            Err(error) => {
                tracing::warn!(%error, "skipping unreadable skeleton store identity");
                continue;
            }
        };
        if !identity.path().is_dir() {
            continue;
        }
        let sessions = match std::fs::read_dir(identity.path()) {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::warn!(store = %identity.path().display(), %error, "skipping unreadable skeleton store");
                continue;
            }
        };
        for session in sessions {
            let path = match session {
                Ok(session) => session.path(),
                Err(error) => {
                    tracing::warn!(%error, "skipping unreadable skeleton store entry");
                    continue;
                }
            };
            let contract = path.join("contract.json");
            match contract.try_exists() {
                Ok(true) => {}
                Ok(false) => continue,
                Err(error) => {
                    tracing::warn!(store = %path.display(), %error, "skipping unreadable skeleton store");
                    continue;
                }
            }
            let metadata = (|| -> Result<_> {
                let value: Value = serde_json::from_slice(&std::fs::read(&contract)?)?;
                if value["kind"] != "Skeleton" {
                    return Ok(None);
                }
                let owner = path.join("author.json");
                let owner: Option<String> = if owner.is_file() {
                    Some(serde_json::from_slice(&std::fs::read(owner)?)?)
                } else {
                    None
                };
                let created: chrono::DateTime<chrono::Utc> =
                    std::fs::metadata(&contract)?.modified()?.into();
                Ok(Some((
                    value["identity"].as_str().unwrap_or_default().to_owned(),
                    owner,
                    created,
                )))
            })();
            match metadata {
                Ok(Some((identity, owner, created))) => {
                    stores.push((path, identity, owner, created))
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(store = %path.display(), %error, "skipping unreadable skeleton store")
                }
            }
        }
    }
    stores.sort_by_key(|(path, _, _, created)| (std::cmp::Reverse(*created), path.clone()));
    let mut authors: Vec<_> = records
        .iter()
        .filter_map(|record| {
            let (subject, _, ordinal) = author_call(&record.call.id)?;
            (subject == "skeleton" && record.invalidated_by.is_none()).then_some((ordinal, record))
        })
        .collect();
    authors.sort_by_key(|(ordinal, _)| std::cmp::Reverse(*ordinal));
    for (_, record) in authors {
        let start = started(record)?;
        let end = chrono::DateTime::parse_from_rfc3339(&record.finished_at).ok();
        for (path, identity, owner, created) in &stores {
            let belongs = match owner {
                Some(call) => call == &record.call.id,
                None => end.is_some_and(|end| *created >= start && *created <= end),
            };
            if !belongs {
                continue;
            }
            let retained = (|| -> Result<Option<Value>> {
                let landing = RecordLanding::open(
                    path.clone(),
                    identity.clone(),
                    RecordKind::Skeleton,
                    vec![],
                    false,
                )?;
                if !landing.has_landed_records()? {
                    return Ok(None);
                }
                landing
                    .retained_completion()?
                    .map(|receipt| landing.compose(&receipt).map_err(Into::into))
                    .transpose()
            })();
            match retained {
                Ok(Some(value)) => {
                    *candidate = serde_json::to_string(&value)?;
                    *candidate_call = record.call.id.clone();
                    return Ok(());
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(store = %path.display(), %error, "skipping unreadable skeleton store")
                }
            }
        }
    }
    Ok(())
}
