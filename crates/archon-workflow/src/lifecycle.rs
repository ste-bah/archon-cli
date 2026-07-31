use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::json;

use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::persistence;
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::store::WorkflowStore;

include!("lifecycle_a.rs");
include!("lifecycle_b.rs");
