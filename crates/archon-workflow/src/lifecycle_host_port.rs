//! The port through which the decomposed-PRD lifecycle driver reaches its host.
//!
//! `LifecycleDriver` is the whole decomposed-PRD lifecycle: dependency waves,
//! implementation fan-out, focused verification, triage, remediation, review,
//! and the terminal gate. All of it is a sequence of host calls and decisions
//! over the results — and the decisions already live in this crate, as
//! [`crate::v2::lifecycle_policy`]. What kept the driver itself in the binary
//! was one field: `Arc<WorkflowScriptHost>`. That type owns a
//! `WorkflowV2ScriptRunner`, which owns the live agent client, which reaches
//! `archon-pipeline` and `archon-tools` — neither of which this crate may
//! depend on.
//!
//! So the direction is inverted, the same way [`crate::llm_client_port`]
//! inverts the LLM and [`crate::ui_sink_port`] inverts the terminal UI. The
//! driver declares the three things it actually asks the host for, the host
//! supplies them, and nothing here names `WorkflowScriptHost`.
//!
//! Three, not five. An earlier survey counted five host reaches from the
//! lifecycle module, but two of them — `summary` and `emit_terminal_status`,
//! plus `mark_script_failure` — are called from the *composition root* that
//! builds the host and hands it to the driver, never from the driver. That root
//! is an inherent `impl WorkflowV2ScriptRunner`; coherence pins it to the crate
//! owning that type, so it stays in the binary along with those three calls.
//! The driver reaches the host through [`LifecycleHost::execute`],
//! [`LifecycleHost::load_call_record`], and
//! [`LifecycleHost::pack_reduce_source`] and nothing else.
//!
//! Unlike the LLM port, no error translation happens at this boundary: every
//! method already speaks [`WorkflowResult`], because the host implementation it
//! was extracted from already did. A host that needs to report a foreign error
//! wraps it with [`WorkflowError::port`](crate::error::WorkflowError::port),
//! which is `#[error(transparent)]`, so its message reaches the user and the
//! driver's marker checks unchanged. That matters here specifically: the driver
//! routes on [`TERMINAL_HOST_CALL_MARKER`] appearing in an error's text.

use async_trait::async_trait;
use serde_json::Value;

use crate::error::WorkflowResult;
use crate::v2::result_store::WorkflowV2CallRecord;

/// Prefix a host uses on the error it raises when a call is terminal — the run
/// is over and no further stage should be attempted.
///
/// The driver and the host both spell this, so it lives with the port rather
/// than with either side. It is matched with `contains`, not equality: the host
/// appends the call id and outcome after it.
pub const TERMINAL_HOST_CALL_MARKER: &str = "workflow terminal host call:";

/// The host a decomposed-PRD lifecycle run drives.
///
/// Futures are `Send`. The driver is awaited on a current-thread runtime inside
/// `spawn_blocking` and never spawns a task of its own, so it does not need
/// `Send` — but the host implementation's own call path does not hold anything
/// across an await that would prevent it, so requiring `Send` costs the host
/// nothing and leaves the driver usable from a spawned task if a caller ever
/// wants that. (Contrast [`crate::llm_client_port::WorkflowLlmClientFactory`],
/// which is deliberately `?Send`: building a client starts MCP servers, and
/// that path holds a `tokio` `Notified` across an await — rust-lang/rust#100013.
/// Nothing on this trait reaches that code.)
#[async_trait]
pub trait LifecycleHost: Send + Sync {
    /// Run one host call. `method` is the [`WorkflowV2HostMethod`] name
    /// (`"reduce"`, `"parallel"`, `"fanout"`, `"finalReport"`, ...) and
    /// `payload` is the JSON envelope `{ id, source?, options }`. The reply is
    /// JSON text.
    ///
    /// Strings, not typed values, because this is the same entry point the
    /// QuickJS bridge called and the recorded call/result identity is defined
    /// over the JSON text. Typing it here would move the serialisation and
    /// change the hashes reuse keys on.
    ///
    /// [`WorkflowV2HostMethod`]: crate::v2::host_api::WorkflowV2HostMethod
    async fn execute(&self, method: String, payload: String) -> WorkflowResult<String>;

    /// The stored record for `call_id`, or `None` if the call left none.
    ///
    /// The driver consults this on exactly one path: a `finalReport` that
    /// failed with [`TERMINAL_HOST_CALL_MARKER`] may still have recorded an
    /// accepted result, and the fallback report is only legitimate when it did
    /// not.
    fn load_call_record(&self, call_id: &str) -> WorkflowResult<Option<WorkflowV2CallRecord>>;

    /// Shrink a reducer source the way the host packs sources for transport.
    ///
    /// Used on the driver's transport retry for reduce ids that have no
    /// verification-specific slimming rule. The packing is the host's — it
    /// strips tool declarations, which is a property of how the host binds MCP
    /// tools, not something this crate can decide.
    fn pack_reduce_source(&self, source: &Value) -> Value;
}
