//! Voice belongs to the interactive startup boundary.
//!
//! Exercise the production boundary without opening an audio device. CLI child
//! startup is covered by voice_toggle_mode_wire_test and main_tests.

#[path = "../src/main_startup.rs"]
mod startup;
use startup::run;

#[path = "../src/main_startup_tests.rs"]
mod tests;
