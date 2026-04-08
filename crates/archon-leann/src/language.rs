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

/// Check if a path matches any of the given exclusion patterns.
///
/// Performs simple component-based matching: if any path component equals
/// one of the patterns, the path is considered excluded.
pub fn is_excluded(path: &Path, patterns: &[String]) -> bool {
    for component in path.components() {
        let s = component.as_os_str().to_string_lossy();
        for pattern in patterns {
            if s == *pattern {
                return true;
            }
        }
    }
    false
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
