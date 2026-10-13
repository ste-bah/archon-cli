use std::path::{Path, PathBuf};

use super::super::workflow_host_command_logic::CAPABILITY_LOGIC;
use super::super::workflow_host_command_logic_closure::Workspace;
use super::super::workflow_host_command_logic_source::hashed_text_for_file;
use super::{Tree, sources_digest};

#[test]
fn logic_361_registry_pins_are_normalized_and_reuse_code_is_hashed() {
    let tree = Tree::new();
    tree.write(
        "src/command/mod.rs",
        "mod root;\nmod workflow_host_command_logic;\n",
    );
    tree.write(
        "src/command/root.rs",
        "pub fn verdict() -> bool { super::workflow_host_command_logic::key_token() && \
         super::workflow_host_command_logic::outcome_logic_holds() }\n",
    );
    let registry = |pin: char, key: bool, outcome: bool| {
        format!(
            "sources_digest: \"{}\",\n\
             pub fn key_token() -> bool {{ {} }}\n\
             pub fn outcome_logic_holds() -> bool {{ {} }}\n",
            pin.to_string().repeat(64),
            key,
            outcome
        )
    };
    let path = "src/command/workflow_host_command_logic.rs";
    tree.write(path, &registry('a', true, true));
    let original = tree.digest();
    tree.write(path, &registry('b', true, true));
    assert_eq!(
        tree.digest(),
        original,
        "changing only a registry pin does not change the measured digest"
    );
    tree.write(path, &registry('b', false, true));
    assert_ne!(tree.digest(), original, "key_token is part of reuse logic");
    tree.write(path, &registry('b', true, false));
    assert_ne!(
        tree.digest(),
        original,
        "outcome_logic_holds is part of reuse logic"
    );
    assert_ne!(
        hashed_text_for_file(
            "src/command/other.rs",
            "sources_digest: \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\n",
        ),
        hashed_text_for_file(
            "src/command/other.rs",
            "sources_digest: \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\n",
        ),
        "the normalization is keyed only to the registry path"
    );
}

#[test]
fn logic_361_registry_is_hashed_by_every_dependent_capability() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut workspace = Workspace::new(&repo);
    let registry = Path::new("src/command/workflow_host_command_logic.rs");
    let dependents = [
        "freeze-acceptance",
        "freeze-skeleton",
        "verify-frozen-acceptance",
        "verify-frozen-skeleton",
    ];
    for id in dependents {
        let logic = CAPABILITY_LOGIC
            .iter()
            .find(|logic| logic.id == id)
            .unwrap();
        let roots = logic.sources.concat();
        let hashed = sources_digest(&mut workspace, &repo, &roots, &[]);
        assert!(
            hashed.closure.files.contains(registry),
            "{id} must hash the executable registry logic"
        );
    }
}
