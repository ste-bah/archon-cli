//! The workflow.js script bridge: one grammar, two hosts.
//!
//! Everything here is shared by the LIVE host and the DRY-RUN recorder, which
//! is why it is one module rather than two. `script_source` composes the same
//! source text for both, `ScriptHostRequest`/`parse_script_options` give both
//! the same typed payload parsing, and the result/reuse helpers are the
//! reduction the live host performs over what comes back. A second
//! interpretation of script text on either side is exactly the drift this
//! sharing exists to prevent.
//!
//! What stayed in the binary: `WorkflowScriptHost` and
//! `WorkflowV2ScriptRunner`. They are the composition root — the only code that
//! builds the concrete host around the live agent client, which reaches
//! `archon-pipeline` and `archon-tools`. They call into this module; nothing
//! here names them.
//!
//! The v3 prelude lives here rather than beside the v3 authoring cluster
//! because `script_source` injects it into every script it composes: it is a
//! dependency of the bridge, not of the author.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use rquickjs::function::{Async, Func};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};

pub(crate) use crate::error::{WorkflowError, WorkflowResult};
pub(crate) use crate::v2::call_execution::WorkflowV2CallExecution;
pub(crate) use crate::v2::host_api::{
    WorkflowV2ArtifactRequirement, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2WriteMode,
};
pub(crate) use crate::v2::lifecycle_driver::is_transport_failure_text;
pub(crate) use crate::v2::result::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2Status, WorkflowV2TaskCoverageStatus,
};
pub(crate) use crate::v2::result_store::{WorkflowV2CallRecord, WorkflowV2TaskCompletionEvidence};
pub(crate) use crate::v2::scheduler::stable_value_hash;

mod dry_run_a;
mod dry_run_b;
mod helpers_a;
mod helpers_b;
mod v3_prelude;
mod verification;

pub use dry_run_a::*;
use dry_run_b::*;
pub use helpers_a::*;
pub use helpers_b::*;
use v3_prelude::*;
use verification::*;

// Two prelude entry points the v3 authoring cluster still reaches from the
// binary: its source validator requires the `export const meta` marker, and its
// tests assert on the normalized script text. Narrow when that cluster follows.
pub use v3_prelude::{normalize_workflow_export, workflow_meta_marker_offset};
