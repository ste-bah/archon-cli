//! Opt-in native acceptance execution policy. No ambient environment inheritance.
use std::{collections::BTreeMap,path::PathBuf};
use serde::{Deserialize,Serialize};

#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceExecutionConfig {
    pub scratch_parent:PathBuf,
    pub project_inputs:Vec<PathBuf>,
    pub project_repository_view:AcceptanceProjectView,
    pub toolchain_path:String,
    #[serde(default)]
    pub environment:BTreeMap<String,String>,
    pub cargo_seed:Option<PathBuf>,
    pub timeout_secs:u64,
    pub output_bytes:usize,
    pub scratch_bytes:u64,
}
#[derive(Clone,Copy,Debug,Default,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum AcceptanceProjectView { #[default] Separate, Combined }
