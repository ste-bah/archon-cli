//! Argument parsing for the `/workflow decompose` and `/workflow resume`
//! slash forms, split from the host so it stays under the line ceiling.

use std::path::PathBuf;

use anyhow::{Result, anyhow};

use super::FixedDecompositionTuiRequest;

pub(crate) fn parse_slash_args(args: &[String]) -> Result<FixedDecompositionTuiRequest> {
    if args.first().is_none_or(|value| value != "decompose") {
        return Err(anyhow!("expected workflow decompose arguments"));
    }
    let mut prd = None;
    let mut tasks = None;
    let mut repository = None;
    let mut index = 1usize;
    while index < args.len() {
        let flag = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| anyhow!("/workflow decompose requires a value after {flag}"))?;
        match flag.as_str() {
            "--prd" if prd.is_none() => prd = Some(PathBuf::from(value)),
            "--tasks" if tasks.is_none() => tasks = Some(PathBuf::from(value)),
            "--repository" if repository.is_none() => repository = Some(PathBuf::from(value)),
            "--prd" | "--tasks" | "--repository" => return Err(anyhow!("duplicate {flag}")),
            other => return Err(anyhow!("unknown /workflow decompose argument {other}")),
        }
        index += 2;
    }
    Ok(FixedDecompositionTuiRequest {
        prd_path: prd.ok_or_else(|| anyhow!("/workflow decompose requires --prd <PATH>"))?,
        task_root: tasks.ok_or_else(|| anyhow!("/workflow decompose requires --tasks <DIR>"))?,
        repository,
    })
}

pub(crate) fn parse_resume_args(args: &[String]) -> Result<Option<String>> {
    if args.first().is_none_or(|value| value != "resume") {
        return Ok(None);
    }
    let values: Vec<&str> = args[1..]
        .iter()
        .map(String::as_str)
        .filter(|value| *value != "--live")
        .collect();
    if values.len() != 1 || values[0].trim().is_empty() {
        return Err(anyhow!("/workflow resume requires exactly one run id"));
    }
    Ok(Some(values[0].to_string()))
}
