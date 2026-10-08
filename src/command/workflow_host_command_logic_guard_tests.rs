//! Issue 361: the guard that keeps logic versions honest. Each capability's
//! digest covers the whole dependency closure of its roots, minus a reviewed
//! denylist; comment lines and in-file test blocks do not move it.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::command::workflow_host_command_logic::{CAPABILITY_LOGIC, LOGIC_DENYLIST};
use crate::command::workflow_host_command_logic_closure::{Closure, Workspace};
use crate::command::workflow_host_command_logic_source::hashed_text;

/// A capability's hashed sources: the digest, the closure, and the
/// references the walk could not resolve.
pub(crate) struct Hashed {
    pub(crate) digest: String,
    pub(crate) closure: Closure,
    pub(crate) errors: BTreeSet<String>,
}

/// The digest of the closure of `roots` under `base`: each file named by
/// its path and read with LF line ends (Rust without comment lines and
/// trailing test blocks), then every item-to-item reference.
pub(crate) fn sources_digest(
    workspace: &mut Workspace,
    base: &Path,
    roots: &[&str],
    deny: &[(&str, &str)],
) -> Hashed {
    let closure = workspace.closure(roots, deny);
    let mut framed = Vec::new();
    let mut frame = |part: &[u8]| {
        framed.extend_from_slice(&(part.len() as u64).to_le_bytes());
        framed.extend_from_slice(part);
    };
    for file in &closure.files {
        let name = file
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let text = std::fs::read_to_string(base.join(file)).unwrap();
        let text = if name.ends_with(".rs") {
            hashed_text(&text)
        } else {
            text.replace("\r\n", "\n")
        };
        frame(name.as_bytes());
        frame(text.as_bytes());
    }
    for (from, at, to, target) in &closure.edges {
        let edge = format!("{}#{at}>{}#{target}", from.display(), to.display());
        frame(edge.replace('\\', "/").as_bytes());
    }
    Hashed {
        digest: archon_workflow::task_set_contract::content_digest(&framed),
        closure,
        errors: std::mem::take(&mut workspace.errors),
    }
}

/// Every capability's hashed sources in this repository, computed once.
static REPOSITORY: LazyLock<BTreeMap<&'static str, Hashed>> = LazyLock::new(|| {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut workspace = Workspace::new(&repo);
    CAPABILITY_LOGIC
        .iter()
        .map(|logic| {
            let roots = logic.sources.concat();
            let hashed = sources_digest(&mut workspace, &repo, &roots, LOGIC_DENYLIST);
            (logic.id, hashed)
        })
        .collect()
});

#[test]
fn logic_361_sources_match_their_pinned_digest() {
    let stale = CAPABILITY_LOGIC
        .iter()
        .filter_map(|logic| {
            let hashed = &REPOSITORY[logic.id];
            let files = hashed.closure.files.len();
            (hashed.digest != logic.sources_digest).then(|| {
                format!(
                    "  {} (logic version {}, {files} files): sources digest is {}, pinned {}",
                    logic.id, logic.version, hashed.digest, logic.sources_digest
                )
            })
        })
        .collect::<Vec<_>>();
    assert!(
        stale.is_empty(),
        "The source of these host command capabilities changed (Issue 361):\n{}\n\
         In src/command/workflow_host_command_logic.rs, for each one: if the change can \
         alter any verdict the subcommand gives, bump its `version` (its next call runs \
         again; every other capability keeps its results); if it cannot, keep the version. \
         Either way set `sources_digest` to the digest above. A file that no verdict can \
         depend on may instead go on LOGIC_DENYLIST, with its reason.",
        stale.join("\n")
    );
}

/// The walk is sound: every reference it meets resolves, and lands on a
/// hashed or a denied file. Every denylist entry is reached and says why.
#[test]
fn logic_361_every_referenced_module_is_hashed_or_denied() {
    let mut reached = BTreeSet::new();
    for (id, hashed) in REPOSITORY.iter() {
        assert!(
            hashed.errors.is_empty(),
            "{id}: references the logic walk cannot resolve (teach the walk, never \
             ignore them):\n{}",
            hashed.errors.iter().cloned().collect::<Vec<_>>().join("\n")
        );
        let closure = &hashed.closure;
        let denied = closure.denied.values().flatten().collect::<BTreeSet<_>>();
        for (from, targets) in &closure.references {
            assert!(
                closure.files.contains(from),
                "{id}: {} not hashed",
                from.display()
            );
            for target in targets {
                assert!(
                    closure.files.contains(target) || denied.contains(target),
                    "{id}: {} references {}, which is neither hashed nor denied",
                    from.display(),
                    target.display()
                );
            }
        }
        reached.extend(closure.denied.keys().cloned());
    }
    for (entry, reason) in LOGIC_DENYLIST {
        assert!(
            reason.len() > 20,
            "{entry}: a denylist entry needs its reason"
        );
        assert!(
            reached.contains(*entry),
            "{entry}: no capability reaches it; remove it"
        );
    }
}

/// The logic changes that landed outside round 1's hand lists now trip the
/// guard of the capabilities whose verdicts they changed.
#[test]
fn logic_361_review_logic_changes_trip_the_guard() {
    let hashes = |id: &str, file: &str| REPOSITORY[id].closure.files.contains(Path::new(file));
    let workflow = "crates/archon-workflow/src";
    // #244 (b844d5971, 4723522cc): the verifier-strength rule.
    for file in [
        "verifier_strength.rs",
        "verifier_strength/prelude.rs",
        "verifier_strength/shell_lexer.rs",
        "verifier_strength/shell_lexer/commands.rs",
        "verifier_strength/shell_lexer/words.rs",
    ] {
        for id in ["task-set-lint", "land-task-body", "freeze-acceptance"] {
            assert!(hashes(id, &format!("{workflow}/{file}")), "{id}: {file}");
        }
    }
    // 1c5cbc2bf: the acceptance-check environment.
    for file in [
        "acceptance_check_environment.rs",
        "acceptance_check_environment_withheld.rs",
        "acceptance_scratch.rs",
        "acceptance_scratch_direct.rs",
        "acceptance_scratch_identity.rs",
        "acceptance_scratch_process.rs",
    ] {
        for id in ["freeze-acceptance", "freeze-skeleton"] {
            assert!(hashes(id, &format!("{workflow}/{file}")), "{id}: {file}");
        }
    }
    // The trace verdict itself, the task parser and the topology crate.
    for file in [
        "crates/archon-knowledge/src/traceability/anchors.rs",
        "crates/archon-knowledge/src/traceability/report.rs",
        "crates/archon-knowledge/src/traceability/falsification/mutate.rs",
        "crates/archon-workflow/src/task_universe_parsing.rs",
        "crates/archon-topology/src/analysis/edge_support.rs",
    ] {
        assert!(hashes("requirements-trace", file), "{file}");
    }
    for file in [
        "crates/archon-workflow/src/task_universe_parsing.rs",
        "crates/archon-workflow/src/task_universe_contract_audit.rs",
        "crates/archon-workflow/src/tool_declarations.rs",
        "src/command/workflow_task_set.rs",
        "src/command/workflow_freeze_candidate.rs",
    ] {
        assert!(hashes("task-set-lint", file), "{file}");
    }
}

/// A small workspace: the binary crate and one library crate.
struct Tree(tempfile::TempDir);

impl Tree {
    fn new() -> Self {
        let tree = Self(tempfile::tempdir().unwrap());
        tree.write("src/main.rs", "mod command;\n");
        tree.write(
            "crates/rules/Cargo.toml",
            "[package]\nname = \"rules-crate\"\n",
        );
        tree
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.0.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    fn hash(&self, deny: &[(&str, &str)]) -> Hashed {
        let mut workspace = Workspace::new(self.0.path());
        sources_digest(
            &mut workspace,
            self.0.path(),
            &["src/command/root.rs"],
            deny,
        )
    }
    fn digest(&self) -> String {
        let hashed = self.hash(&[]);
        assert!(hashed.errors.is_empty(), "{:?}", hashed.errors);
        hashed.digest
    }
    fn files(&self) -> BTreeSet<String> {
        let hashed = self.hash(&[]);
        let files = hashed.closure.files.iter();
        files
            .map(|f| f.to_string_lossy().replace('\\', "/"))
            .collect()
    }
}

#[test]
fn logic_361_the_closure_follows_use_paths_and_impls_across_crates() {
    let tree = Tree::new();
    tree.write(
        "src/command/mod.rs",
        "mod root;\nmod helper;\nmod unused;\n",
    );
    tree.write(
        "src/command/root.rs",
        "use rules_crate::strength::judge;\n\
         pub fn verdict() -> bool { judge() && crate::command::helper::Rule::new().holds() }\n",
    );
    tree.write(
        "src/command/helper.rs",
        "pub struct Rule;\nmod rule_impl;\n",
    );
    tree.write(
        "src/command/helper/rule_impl.rs",
        "impl super::Rule { pub fn new() -> Self { Self } pub fn holds(&self) -> bool { true } }\n",
    );
    tree.write("src/command/unused.rs", "pub fn other() {}\n");
    tree.write("crates/rules/src/lib.rs", "pub mod strength;\n");
    tree.write(
        "crates/rules/src/strength.rs",
        "pub fn judge() -> bool { true }\n",
    );
    let first = tree.digest();
    // A `use` into another workspace crate is followed.
    assert!(tree.files().contains("crates/rules/src/strength.rs"));
    tree.write(
        "crates/rules/src/strength.rs",
        "pub fn judge() -> bool { false }\n",
    );
    let second = tree.digest();
    assert_ne!(
        second, first,
        "a stricter rule in another crate trips the guard"
    );
    // An inline path names a type; its impl block in a child module is hashed.
    tree.write(
        "src/command/helper/rule_impl.rs",
        "impl super::Rule { pub fn new() -> Self { Self } pub fn holds(&self) -> bool { false } }\n",
    );
    let third = tree.digest();
    assert_ne!(third, second, "an impl of a named type is logic");
    // A module no reached item names is not logic.
    tree.write("src/command/unused.rs", "pub fn other() { let _ = 1; }\n");
    assert_eq!(tree.digest(), third);
    assert!(!tree.files().contains("src/command/unused.rs"));
}

#[test]
fn logic_361_the_closure_follows_reexports_and_included_files() {
    let tree = Tree::new();
    tree.write("src/command/mod.rs", "mod root;\n");
    tree.write(
        "src/command/root.rs",
        "const PROMPT: &str = include_str!(\"root_prompt.md\");\n\
         include!(\"root_body.rs\");\n\
         pub fn verdict() -> bool { rules_crate::judge() && !PROMPT.is_empty() }\n\
         pub fn report() -> bool { rules_crate::lax_judge() }\n",
    );
    tree.write("src/command/root_prompt.md", "Judge strictly.\n");
    tree.write("src/command/root_body.rs", "pub fn body() {}\n");
    tree.write(
        "crates/rules/src/lib.rs",
        "mod strict;\nmod lax;\npub use strict::judge;\npub use lax::judge as lax_judge;\n",
    );
    tree.write(
        "crates/rules/src/strict.rs",
        "pub fn judge() -> bool { true }\n",
    );
    tree.write(
        "crates/rules/src/lax.rs",
        "pub fn judge() -> bool { false }\npub fn keep() { crate::strict::judge(); }\n",
    );
    let first = tree.digest();
    let files = tree.files();
    // A re-export is followed to the item it names.
    assert!(files.contains("crates/rules/src/strict.rs"), "{files:?}");
    // Re-pointing a re-export moves the digest even when no hashed file did.
    tree.write(
        "crates/rules/src/lib.rs",
        "mod strict;\nmod lax;\npub use lax::judge;\npub use strict::judge as lax_judge;\n",
    );
    let second = tree.digest();
    assert_ne!(second, first, "the verdict now runs another rule");
    // An included prompt and an included source are logic.
    tree.write("src/command/root_prompt.md", "Judge leniently.\n");
    let third = tree.digest();
    assert_ne!(third, second);
    tree.write("src/command/root_body.rs", "pub fn body() { let _ = 2; }\n");
    assert_ne!(tree.digest(), third);
}

#[test]
fn logic_361_the_closure_stops_at_the_denylist_and_flags_what_it_cannot_resolve() {
    let tree = Tree::new();
    tree.write("src/command/mod.rs", "mod root;\nmod transport;\n");
    tree.write(
        "src/command/root.rs",
        "pub fn verdict() -> bool { super::transport::send() }\n\
         #[cfg(test)]\nfn helper() { crate::command::transport::missing(); }\n",
    );
    tree.write(
        "src/command/transport.rs",
        "pub fn send() -> bool { true }\n",
    );
    let deny = [("src/command/transport.rs", "transport only")];
    let first = tree.hash(&deny);
    assert!(
        first.errors.is_empty(),
        "test code is not walked: {:?}",
        first.errors
    );
    assert!(
        !first
            .closure
            .files
            .contains(Path::new("src/command/transport.rs"))
    );
    assert_eq!(first.closure.denied.len(), 1);
    tree.write(
        "src/command/transport.rs",
        "pub fn send() -> bool { false }\n",
    );
    assert_eq!(
        tree.hash(&deny).digest,
        first.digest,
        "a denied file is not logic"
    );
    // A workspace path that names nothing is an error, never a silent gap.
    tree.write(
        "src/command/root.rs",
        "pub fn verdict() -> bool { super::transport::renamed() }\n",
    );
    let broken = tree.hash(&[]);
    assert!(
        broken.errors.iter().any(|error| error.contains("renamed")),
        "{:?}",
        broken.errors
    );
}

/// A root covers its whole module subtree, `#[path]` modules and new
/// submodules included, and never a test module.
#[test]
fn logic_361_a_root_covers_its_module_subtree_and_no_test() {
    let tree = Tree::new();
    tree.write("src/command/mod.rs", "mod root;\n");
    tree.write(
        "src/command/root.rs",
        "#[path = \"root_helper.rs\"]\nmod helper;\n#[cfg(test)]\nmod tests;\n\
         #[cfg(test)] #[path = \"root_more_tests.rs\"] mod more;\npub(crate) mod inner;\n",
    );
    tree.write(
        "src/command/root_helper.rs",
        "pub fn verdict() -> bool { true }\n",
    );
    tree.write("src/command/root/inner.rs", "pub fn rule() {}\n");
    tree.write("src/command/root/tests.rs", "#[test] fn t() {}\n");
    tree.write("src/command/root_more_tests.rs", "#[test] fn u() {}\n");
    let first = tree.digest();
    let expected = [
        "src/command/root.rs",
        "src/command/root/inner.rs",
        "src/command/root_helper.rs",
    ];
    assert_eq!(tree.files(), expected.map(String::from).into());
    tree.write(
        "src/command/root/tests.rs",
        "#[test] fn t() { assert!(true) }\n",
    );
    tree.write(
        "src/command/root_more_tests.rs",
        "#[test] fn u() { assert!(true) }\n",
    );
    assert_eq!(tree.digest(), first, "a test edit is not a logic change");
    tree.write(
        "src/command/root_helper.rs",
        "pub fn verdict() -> bool { false }\n",
    );
    let second = tree.digest();
    assert_ne!(second, first, "a helper edit is a logic change");
    tree.write(
        "src/command/root/inner.rs",
        "mod deeper;\npub fn rule() {}\n",
    );
    tree.write("src/command/root/inner/deeper.rs", "pub fn stricter() {}\n");
    let third = tree.digest();
    assert!(tree.files().contains("src/command/root/inner/deeper.rs"));
    assert_ne!(third, second);
    tree.write(
        "src/command/root/inner/deeper.rs",
        "pub fn stricter() {}\r\n",
    );
    assert_eq!(tree.digest(), third, "line ends are not logic");
}

#[test]
fn logic_361_comment_lines_and_test_blocks_are_not_logic() {
    let tree = Tree::new();
    tree.write("src/command/mod.rs", "mod root;\n");
    let root = |comment: &str, test: &str, prompt: &str| {
        format!(
            "//! {comment}\n/// {comment}\npub fn verdict() -> bool {{\n    // {comment}\n    \
             PROMPT.len() > 3\n}}\nconst PROMPT: &str = r\"\n// {prompt}\n\";\n\n\
             #[cfg(test)]\nmod tests {{\n    #[test]\n    fn t() {{ {test} }}\n}}\n"
        )
    };
    tree.write("src/command/root.rs", &root("a", "", "rule"));
    let first = tree.digest();
    tree.write("src/command/root.rs", &root("b", "", "rule"));
    assert_eq!(tree.digest(), first, "a comment edit is not a logic change");
    tree.write("src/command/root.rs", &root("b", "assert!(true);", "rule"));
    assert_eq!(tree.digest(), first, "a test edit is not a logic change");
    // A `//` line inside a string literal is text the verdict reads.
    tree.write(
        "src/command/root.rs",
        &root("b", "assert!(true);", "other rule"),
    );
    let prompt = tree.digest();
    assert_ne!(prompt, first);
    // A test block that code follows is not cut.
    let mut text = root("b", "", "other rule");
    text.push_str("pub fn after() {}\n");
    tree.write("src/command/root.rs", &text);
    let followed = tree.digest();
    tree.write(
        "src/command/root.rs",
        &text.replace("fn t() {  }", "fn t() { 1; }"),
    );
    assert_ne!(tree.digest(), followed);
}
