use archon_core::skills::builtin::register_builtins;

#[test]
fn phase1_skills_register_in_builtin_registry() {
    let registry = register_builtins();
    assert!(registry.resolve("to-prd").is_some());
    assert!(registry.resolve("prd-to-spec").is_some());
    assert!(registry.resolve("prd").is_some());
    assert!(registry.resolve("decompose-prd").is_some());
}
