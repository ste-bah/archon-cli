//! Compilation tests for archon-leann crate skeleton.
//! If any `pub mod` declaration is missing or its file doesn't exist, this won't compile.

#[test]
fn leann_modules_are_reachable() {
    use archon_leann::indexer as _;
    use archon_leann::chunker as _;
    use archon_leann::search as _;
    use archon_leann::queue as _;
    use archon_leann::language as _;
    use archon_leann::metadata as _;
    use archon_leann::stats as _;
}
