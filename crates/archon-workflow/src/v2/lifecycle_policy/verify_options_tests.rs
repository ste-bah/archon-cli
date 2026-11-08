use crate::v2::deliverable_contract;

use super::verify_options::{verification_options, write_wave_parallelism};

#[path = "verify_options_tests_a.rs"]
mod verify_options_tests_a;
use verify_options_tests_a::*;
#[path = "verify_options_tests_b.rs"]
mod verify_options_tests_b;
use verify_options_tests_b::*;
#[path = "verify_options_tests_c.rs"]
mod verify_options_tests_c;
#[path = "verify_options_tests_d.rs"]
mod verify_options_tests_d;

fn prepare_verification_items(
    items: Vec<serde_json::Value>,
    root: Option<&str>,
    evidence: &[serde_json::Value],
    universe: &serde_json::Value,
) -> Vec<serde_json::Value> {
    crate::v2::deliverable_native_tests::initialize();
    super::verify_options::prepare_verification_items(items, root, evidence, universe)
}
