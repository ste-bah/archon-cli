use std::fs;
use std::path::Path;

use crate::error::{WorkflowError, WorkflowResult};
use crate::task_universe::WorkflowV2DeliverableContract;

use super::contract_roots::ContractRoots;
use super::declarative_floor::DeclarativeFloorFacts;

/// Collect the filesystem facts consumed by the pure declarative-floor kernel.
///
/// This function reads only host-resolved declared artifact and registry paths,
/// each resolved under the first of `roots` it exists beneath (Issue-22). It
/// never renders or executes a verifier command. Missing, empty, and invalid
/// files are represented as facts so policy evaluation remains deterministic;
/// only an actual filesystem read failure is operational.
pub fn collect_declarative_floor_facts(
    roots: &ContractRoots,
    contract: &WorkflowV2DeliverableContract,
) -> WorkflowResult<DeclarativeFloorFacts> {
    let artifact_path = roots.resolve(&contract.artifact_path);
    let (artifact_present, artifact_byte_len, artifact_bytes) = read_optional(&artifact_path)?;
    let artifact_json =
        if artifact_present && artifact_byte_len > 0 && artifact_format(contract) == "json" {
            artifact_bytes
                .as_deref()
                .and_then(|bytes| serde_json::from_slice(bytes).ok())
        } else {
            None
        };
    let registry_json = match contract.registry_path.as_deref() {
        Some(path) => {
            let path = roots.resolve(path);
            let (present, byte_len, bytes) = read_optional(&path)?;
            if present && byte_len > 0 {
                bytes
                    .as_deref()
                    .and_then(|bytes| serde_json::from_slice(bytes).ok())
            } else {
                None
            }
        }
        None => None,
    };
    Ok(DeclarativeFloorFacts {
        artifact_present,
        artifact_byte_len,
        artifact_json,
        registry_json,
        instance_count: usize::from(artifact_present),
        searched_roots: roots.ordered().map(str::to_string).collect(),
    })
}

fn read_optional(path: &Path) -> WorkflowResult<(bool, u64, Option<Vec<u8>>)> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((false, 0, None));
        }
        Err(source) => {
            return Err(WorkflowError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if !metadata.is_file() {
        return Ok((false, 0, None));
    }
    let byte_len = metadata.len();
    let bytes = fs::read(path).map_err(|source| WorkflowError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok((true, byte_len, Some(bytes)))
}

fn artifact_format(contract: &WorkflowV2DeliverableContract) -> String {
    contract
        .artifact_format
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| {
            if contract.artifact_path.ends_with(".json")
                || contract.artifact_path.ends_with(".jsonl")
            {
                "json".to_string()
            } else {
                "text".to_string()
            }
        })
}
