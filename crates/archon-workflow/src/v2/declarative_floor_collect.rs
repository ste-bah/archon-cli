//! Collect evidence in dependency order: primary files, references, instances.
use super::contract_roots::ContractRoots;
use super::declarative_floor_collect_refs::{instance_records, referenced_files};
use super::declarative_floor_facts::{
    DeclarativeFloorFacts, FileFact, FileState, InstanceBinding, InstanceFact,
};
use super::declarative_floor_instances::{records_field, source_path};
use crate::error::WorkflowResult;
use crate::task_universe::WorkflowV2DeliverableContract;
use std::{fs, path::Path};

pub fn collect_declarative_floor_facts(
    roots: &ContractRoots,
    contract: &WorkflowV2DeliverableContract,
) -> WorkflowResult<DeclarativeFloorFacts> {
    let artifact = read(roots, &contract.artifact_path);
    let artifact_json =
        (super::declarative_floor_values::artifact_format(contract, &contract.artifact_path)
            == "json")
            .then(|| parse(&artifact))
            .flatten();
    let mut facts = DeclarativeFloorFacts {
        artifact_present: matches!(
            &artifact.state,
            FileState::Bytes(_) | FileState::Empty | FileState::DirectoryEvidence(1..)
        ),
        artifact_byte_len: match &artifact.state {
            FileState::Bytes(bytes) => bytes.len() as u64,
            FileState::DirectoryEvidence(n) => *n,
            _ => 0,
        },
        artifact_json,
        searched_roots: roots.ordered().map(str::to_owned).collect(),
        verified_at: Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)),
        ..Default::default()
    };
    facts.files.insert(contract.artifact_path.clone(), artifact);
    if let Some(path) = &contract.registry_path {
        let fact = read(roots, path);
        facts.registry_json = parse(&fact);
        facts.files.insert(path.clone(), fact);
    }
    for path in referenced_files(
        contract,
        facts.artifact_json.as_ref(),
        facts.registry_json.as_ref(),
    ) {
        facts
            .files
            .entry(path.clone())
            .or_insert_with(|| read(roots, &path));
    }
    if contract.artifact_path.contains('<') && contract.artifact_path.contains('>') {
        let references = if contract.instance_artifact_field.is_some()
            && records_field(contract).is_some()
            && source_path(contract).is_some()
        {
            let source = source_path(contract)
                .and_then(|path| facts.files.get(path))
                .and_then(parse);
            let records = source
                .as_ref()
                .map(|source| instance_records(contract, source))
                .unwrap_or_default();
            let paths = records
                .iter()
                .filter_map(|(_, reference)| reference.clone())
                .collect();
            facts.instance_binding = Some(InstanceBinding::SourceBound { records });
            paths
        } else if contract.min_instances >= 1 {
            let (pattern_matches, root) = match glob_instances(roots, &contract.artifact_path) {
                Ok(binding) => binding,
                Err(error) => {
                    facts.collection_findings.push(error);
                    (Vec::new(), None)
                }
            };
            facts.instance_binding = Some(InstanceBinding::Glob {
                pattern_matches: pattern_matches.clone(),
                root,
            });
            pattern_matches
        } else {
            Vec::new()
        };
        facts.instances = references
            .into_iter()
            .map(|reference| InstanceFact {
                fact: read(roots, &reference),
                reference,
            })
            .collect();
    }
    facts.instance_count = if facts.instance_binding.is_some() {
        facts.instances.len()
    } else {
        usize::from(facts.artifact_present)
    };
    Ok(facts)
}

fn parse(fact: &FileFact) -> Option<serde_json::Value> {
    match &fact.state {
        FileState::Bytes(bytes) => super::declarative_floor_json::parse(bytes).ok(),
        _ => None,
    }
}

fn read(roots: &ContractRoots, reference: &str) -> FileFact {
    let path = roots.resolve(reference);
    let mut fact = FileFact {
        shown: path.to_string_lossy().replace('\\', "/"),
        state: FileState::Absent,
    };
    fact.state = match fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if roots.ordered().count() > 1 && !Path::new(reference).has_root() {
                fact.shown = format!("{reference} (looked under {})", roots.described());
            }
            FileState::Absent
        }
        Err(error) => FileState::Unreadable(error.to_string()),
        Ok(metadata) if metadata.is_dir() => match fs::read_dir(&path) {
            Ok(_) => FileState::DirectoryEvidence(
                super::artifact_path_guard::artifact_evidence_byte_len(&path).unwrap_or_default(),
            ),
            Err(error) => FileState::Unreadable(error.to_string()),
        },
        Ok(metadata) if metadata.is_file() => match fs::read(&path) {
            Ok(bytes) if bytes.is_empty() => FileState::Empty,
            Ok(bytes) => FileState::Bytes(bytes),
            Err(error) => FileState::Unreadable(error.to_string()),
        },
        Ok(_) => FileState::Absent,
    };
    fact
}

fn glob_instances(
    roots: &ContractRoots,
    declared: &str,
) -> Result<(Vec<String>, Option<String>), String> {
    let pattern = regex::Regex::new(r"<[^>]+>")
        .expect("template pattern")
        .replace_all(declared, "*");
    let options = glob::MatchOptions {
        require_literal_leading_dot: true,
        case_sensitive: !cfg!(windows),
        ..Default::default()
    };
    for root in roots.ordered() {
        let candidate = if Path::new(pattern.as_ref()).has_root() {
            pattern.to_string()
        } else {
            format!(
                "{}/{}",
                glob::Pattern::escape(&root.replace('\\', "/")),
                pattern
            )
        };
        let matches = glob::glob_with(&candidate, options)
            .map_err(|error| format!("declared instance pattern is invalid: {declared}: {error}"))?
            .map(|entry| {
                entry
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
                    .map_err(|error| format!("declared instance cannot be read: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !matches.is_empty() {
            let mut matches = matches;
            matches.sort();
            return Ok((matches, Some(root.to_owned())));
        }
        if Path::new(pattern.as_ref()).has_root() {
            break;
        }
    }
    Ok((Vec::new(), None))
}
