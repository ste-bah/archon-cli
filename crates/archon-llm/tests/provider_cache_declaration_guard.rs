//! Guard: every real provider must *declare* how it caches prompts.
//!
//! `LlmProvider::cache_strategy` and `LlmProvider::cache_platform` both have
//! trait defaults (`CacheStrategy::None` / `CachePlatform::Unknown`). That is a
//! safe default but a silent one: three providers inherited it by accident and
//! nothing failed — requests simply went out uncached, which on Bedrock bills
//! full price on every turn. A textual guard is the only thing that catches the
//! omission, because "inherited None" and "deliberately None" are identical at
//! runtime.
//!
//! Adding a provider? Implement both methods on it. `None`/`Unknown` are
//! legitimate answers — this test only insists that you make the choice.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Provider modules known to implement `LlmProvider`, relative to
/// `crates/archon-llm/src/providers/`. Kept explicit so that moving a provider
/// out of this directory trips the guard instead of quietly escaping it.
const EXPECTED_PROVIDER_FILES: &[&str] = &[
    "anthropic.rs",
    "bedrock.rs",
    "codex/client.rs",
    "local.rs",
    "openai.rs",
    "openai_compat.rs",
    "vertex.rs",
];

fn providers_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("providers")
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("providers directory is readable") {
        let path = entry.expect("directory entry is readable").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Path relative to `src/providers/`, with forward slashes so the expectations
/// above read the same on Windows and Unix.
fn relative_key(path: &Path) -> String {
    path.strip_prefix(providers_dir())
        .expect("path is under src/providers")
        .to_string_lossy()
        .replace('\\', "/")
}

fn provider_impl_files() -> BTreeSet<String> {
    let mut sources = Vec::new();
    rust_sources(&providers_dir(), &mut sources);

    sources
        .into_iter()
        .filter(|path| {
            let body = std::fs::read_to_string(path).expect("source file is valid UTF-8");
            body.contains("impl LlmProvider for")
        })
        .map(|path| relative_key(&path))
        .collect()
}

#[test]
fn every_provider_declares_its_cache_strategy() {
    let mut missing = Vec::new();

    for key in provider_impl_files() {
        let body =
            std::fs::read_to_string(providers_dir().join(&key)).expect("source file is readable");

        let mut gaps = Vec::new();
        if !body.contains("fn cache_strategy(") {
            gaps.push("cache_strategy");
        }
        if !body.contains("fn cache_platform(") {
            gaps.push("cache_platform");
        }
        if !gaps.is_empty() {
            missing.push(format!("{key}: missing {}", gaps.join(", ")));
        }
    }

    assert!(
        missing.is_empty(),
        "these providers inherit the LlmProvider caching defaults instead of declaring \
         their own, so archon cannot know whether they cache:\n  {}\n\
         Implement both methods. `CacheStrategy::None` / `CachePlatform::Unknown` are \
         valid answers — say so explicitly.",
        missing.join("\n  ")
    );
}

#[test]
fn provider_module_layout_is_unchanged() {
    let found = provider_impl_files();
    let expected: BTreeSet<String> = EXPECTED_PROVIDER_FILES
        .iter()
        .map(|name| (*name).to_string())
        .collect();

    assert_eq!(
        found, expected,
        "the set of files implementing LlmProvider under src/providers/ changed. \
         Update EXPECTED_PROVIDER_FILES, and make sure the new or moved provider \
         declares cache_strategy and cache_platform."
    );
}

/// The decorators wrap every provider archon actually runs. If one of them
/// stopped forwarding, caching would die everywhere at once while each
/// underlying provider still looked correct in isolation.
#[test]
fn decorators_forward_cache_declarations() {
    let crate_src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    for (file, inner) in [("retry.rs", "self.inner"), ("active.rs", "self.current()")] {
        let body = std::fs::read_to_string(crate_src.join(file)).expect("decorator is readable");

        for method in ["cache_strategy", "cache_platform"] {
            assert!(
                body.contains(&format!("{inner}.{method}(")),
                "{file} does not forward {method} to the wrapped provider — caching \
                 would be silently disabled for every provider behind this decorator"
            );
        }
    }
}
