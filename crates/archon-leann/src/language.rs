//! Language detection and tree-sitter grammars.

use std::path::Path;

/// Detect the programming language of a file based on its extension.
///
/// Returns `None` if the extension is not recognized.
pub fn detect_language(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;
    let lang = match ext {
        "rs" => "rust",
        "py" => "python",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "js" => "javascript",
        "jsx" => "javascriptreact",
        "go" => "go",
        "java" => "java",
        "c" => "c",
        "cpp" | "cc" | "cxx" => "cpp",
        "h" => "c",
        "hpp" | "hxx" | "hh" => "cpp",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "scala" => "scala",
        "cs" => "csharp",
        "lua" => "lua",
        "sh" | "bash" | "zsh" => "shell",
        "sql" => "sql",
        "md" | "markdown" => "markdown",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "r" | "R" => "r",
        "dart" => "dart",
        "ex" | "exs" => "elixir",
        "erl" => "erlang",
        "hs" => "haskell",
        "ml" | "mli" => "ocaml",
        "pl" | "pm" => "perl",
        "zig" => "zig",
        "nim" => "nim",
        "v" => "v",
        _ => return None,
    };
    Some(lang.to_string())
}

/// Whether a detected language is source code the indexer should chunk.
///
/// [`detect_language`] also names markup and data formats (`markdown`, `json`,
/// `yaml`, `toml`, `sql`) because other callers want them; the code index does
/// not. Kept next to `detect_language` so the two lists are read together --
/// adding an extension without adding it here silently indexes nothing.
pub fn is_code_language(language: &str) -> bool {
    matches!(
        language,
        "rust"
            | "python"
            | "typescript"
            | "typescriptreact"
            | "javascript"
            | "javascriptreact"
            | "go"
            | "java"
            | "c"
            | "cpp"
            | "ruby"
            | "php"
            | "swift"
            | "kotlin"
            | "scala"
            | "csharp"
            | "lua"
            | "shell"
            | "r"
            | "dart"
            | "elixir"
            | "erlang"
            | "haskell"
            | "ocaml"
            | "perl"
            | "zig"
            | "nim"
            | "v"
    )
}

/// Check whether a repository-relative path matches any include glob.
///
/// An empty pattern list includes everything, so a caller that never set
/// `include_patterns` keeps the language check as its only filter.
///
/// Matching is on the relative path with `/` separators, because the patterns
/// callers write (`**/*.rs`) are repo-relative and slash-spelled while the walk
/// hands us absolute Windows paths. The bare file name is tried as well so a
/// caller can write `*.rs` and mean it -- glob's `**/` needs a separator to
/// match against, which a file at the repository root does not have.
pub fn is_included(relative: &Path, patterns: &[glob::Pattern]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    let normalized = relative.to_string_lossy().replace('\\', "/");
    let file_name = relative.file_name().and_then(|name| name.to_str());
    patterns.iter().any(|pattern| {
        pattern.matches(&normalized) || file_name.is_some_and(|n| pattern.matches(n))
    })
}

/// Check if a path matches any of the given exclusion patterns.
///
/// Performs component-based matching: if any path component equals one of the
/// patterns, the path is excluded. Patterns are therefore directory *names*
/// (`target`), not globs.
///
/// Glob-shaped patterns are normalised rather than ignored. A caller that
/// passes `**/target/**` — which no path component can ever equal — otherwise
/// excludes nothing at all, silently, and the indexer walks `target/`,
/// `node_modules/` and `.git/`. That is not hypothetical: it is what
/// `LeannIntegration::init_repository_blocking_with_cancel` did, and on a Rust
/// repository it turned a small corpus into tens of gigabytes of build output
/// with no error and no log line to say so. Accepting both spellings costs one
/// trim and removes a class of silent misconfiguration.
pub fn is_excluded(path: &Path, patterns: &[String]) -> bool {
    for component in path.components() {
        let s = component.as_os_str().to_string_lossy();
        for pattern in patterns {
            if s == normalize_exclude_pattern(pattern) {
                return true;
            }
        }
    }
    false
}

/// Reduce a glob-shaped exclusion to the directory name it names.
///
/// `**/target/**` → `target`, `target/**` → `target`, `target` → `target`.
/// A pattern with interior globbing (`src/**/gen`) is left alone: it will not
/// match a component, which is the honest outcome for something this matcher
/// cannot express, rather than guessing at intent.
pub fn normalize_exclude_pattern(pattern: &str) -> &str {
    pattern
        .trim_start_matches("**/")
        .trim_end_matches("/**")
        .trim_matches('/')
}

/// Returns the default set of directory names to exclude from indexing.
pub fn default_exclude_patterns() -> Vec<String> {
    vec![
        "node_modules".to_string(),
        "target".to_string(),
        ".git".to_string(),
        "__pycache__".to_string(),
        ".venv".to_string(),
        "dist".to_string(),
        "build".to_string(),
        "coverage".to_string(),
        ".tv".to_string(),
        ".archon".to_string(),
        ".claude".to_string(), // backward compat exclusion
        "site-packages".to_string(),
    ]
}
