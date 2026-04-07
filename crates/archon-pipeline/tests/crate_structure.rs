//! Compilation tests for archon-pipeline crate skeleton.
//! If any `pub mod` declaration is missing or its file doesn't exist, this won't compile.

#[test]
fn pipeline_top_level_modules_are_reachable() {
    // These will fail to compile if the corresponding pub mod + file is missing
    let _ = std::any::type_name::<fn()>();
    use archon_pipeline::runner as _;
    use archon_pipeline::session as _;
    use archon_pipeline::retry as _;
    use archon_pipeline::prompt_cap as _;
    use archon_pipeline::memory as _;
}

#[test]
fn pipeline_sub_modules_are_reachable() {
    use archon_pipeline::coding as _;
    use archon_pipeline::research as _;
    use archon_pipeline::learning as _;
    use archon_pipeline::kb as _;
}
