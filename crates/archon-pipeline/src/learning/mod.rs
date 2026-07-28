//! Learning and neural systems.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

pub(crate) fn run_script_guarded(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
) -> Result<NamedRows> {
    archon_cozo::run_bound_script_guarded(db, script, params, mutability, context)
}

pub mod causal;
pub mod confidence;
pub mod desc;
pub mod gnn;
pub mod integration;
pub mod migrations;
pub mod modes;
pub mod patterns;
pub mod provenance;
pub mod reasoning;
pub mod reflexion;
pub mod schema;
pub mod shadow;
pub mod sona;
pub mod trajectory_store;
