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
//! The v3 dialect sits alongside it: the primitive prelude `script_source`
//! injects into every script it composes, the reference the author agent is
//! handed, and the pre-flight that refuses an authored script which would plan
//! no real work. That pre-flight IS a dry run, which is why it belongs to the
//! bridge and not to the v3 composition root.

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
pub(crate) use crate::v2::result_store::{
    WorkflowV2CallRecord, WorkflowV2ResultStore, WorkflowV2TaskCompletionEvidence,
};
pub(crate) use crate::v2::scheduler::stable_value_hash;

mod dry_run_a;
mod dry_run_b;
mod helpers_a;
mod helpers_b;
mod v3_author_a;
mod v3_author_b;
mod v3_author_checks_a;
mod v3_author_checks_b;
mod v3_prelude;
mod verification;

pub use dry_run_a::*;
use dry_run_b::*;
pub use helpers_a::*;
pub use helpers_b::*;
pub use v3_author_a::*;
pub use v3_author_b::*;
pub use v3_author_checks_a::*;
pub use v3_author_checks_b::*;
use v3_prelude::*;
use verification::*;

// One prelude entry point outside this module: the binary's tests assert on the
// normalized script text. The dialect's own callers are all in here.
pub use v3_prelude::normalize_workflow_export;

#[cfg(test)]
#[path = "v3_author_checks_tests.rs"]
mod v3_author_checks_tests;
