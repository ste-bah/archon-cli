//! Canary regression for run `wf-afae6bee` (PRD-TRADING-DATA-LAKE-AHDM-001).
//!
//! The original run blocked at `blocked-verification-failed-1` on TASK-TDL-001:
//! the task pack required artifact evidence under `.archon/artifacts/TASK-TDL-001/`,
//! verification demanded it, but no implementing agent was ever instructed to
//! write it. Verification repair loops (1-1, 1-2, 1-3) retried the same failing
//! check and the run latched terminally.
//!
//! This test reconstructs that run with a scripted agent client whose single
//! honesty rule is: **an agent writes the required artifact if and only if the
//! prompt it receives names the artifact path**. The test passes only when the
//! runtime carries the declared artifact requirement from the task pack into
//! the implementing agent's instructions and the run finishes with a final
//! report instead of a run-level verification block.
//!
//! Failed on the pre-rescue architecture (inferred artifact contracts,
//! terminal latch); GREEN since rescue Phase 3 (declared artifact contracts:
//! task-pack declarations reach the implementing agent, write capability is
//! declared not role-sniffed, artifact paths resolve against the project
//! root, and the completion ledger receives write-fanout evidence).
//! This is the rescue's acceptance test — it must stay green.

use std::path::PathBuf;
use std::process::Command as CanaryGitCommand;
use std::sync::Arc;
use std::sync::Mutex as CanaryMutex;
use std::sync::OnceLock;

use anyhow::Result as CanaryResult;
use archon_pipeline::runner::{LlmClient, LlmResponse};
use archon_workflow::CommandAction;

use super::{LiveApprovalMode, run_live_action};

#[path = "workflow_live_canary_tests_a.rs"]
mod workflow_live_canary_tests_a;
use workflow_live_canary_tests_a::*;
#[path = "workflow_live_canary_tests_b.rs"]
mod workflow_live_canary_tests_b;
use workflow_live_canary_tests_b::*;
#[path = "workflow_live_canary_tests_c.rs"]
mod workflow_live_canary_tests_c;
use workflow_live_canary_tests_c::*;
