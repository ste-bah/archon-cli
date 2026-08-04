use crate::v2::deliverable_contract;

use super::verify_options::{
    prepare_verification_items, verification_options, write_wave_parallelism,
};

#[path = "verify_options_tests_a.rs"]
mod verify_options_tests_a;
use verify_options_tests_a::*;
#[path = "verify_options_tests_b.rs"]
mod verify_options_tests_b;
use verify_options_tests_b::*;
#[path = "verify_options_tests_c.rs"]
mod verify_options_tests_c;
use verify_options_tests_c::*;
#[path = "verify_options_tests_d.rs"]
mod verify_options_tests_d;
use verify_options_tests_d::*;
