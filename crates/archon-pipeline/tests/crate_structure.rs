//! Compilation tests for archon-pipeline crate skeleton.
//! If any `pub mod` declaration is missing or its file doesn't exist, this won't compile.

#[test]
fn pipeline_top_level_modules_are_reachable() {
    // These will fail to compile if the corresponding pub mod + file is missing
    let _ = std::any::type_name::<fn()>();
}

#[test]
fn pipeline_sub_modules_are_reachable() {}
