//! Symbol-aware code chunker.
//!
//! Uses tree-sitter grammars to split source files into semantically meaningful
//! chunks (one per top-level symbol). Falls back to line-based splitting for
//! unknown languages or when no symbols are found.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::metadata::{CodeChunk, CodeMetadata};

// ---------------------------------------------------------------------------
// Language enum
// ---------------------------------------------------------------------------

/// Programming languages supported by the chunker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Language {
    Go,
    Python,
    Rust,
    TypeScript,
    Unknown,
}

// ---------------------------------------------------------------------------
// Chunker
// ---------------------------------------------------------------------------

/// Symbol-aware code chunker backed by tree-sitter grammars.
pub struct Chunker {
    languages: HashMap<Language, tree_sitter::Language>,
}

impl Chunker {
    /// Create a new chunker.
    ///
    /// If `grammar_dir` is `Some`, the chunker will attempt to load dynamic
    /// grammars (`.so` / `.dylib`) from that directory.  A missing or
    /// unreadable directory is **not** an error — built-in grammars are always
    /// registered regardless.
    pub fn new(grammar_dir: Option<PathBuf>) -> Result<Self> {
        let mut languages = HashMap::new();

        // Optionally try dynamic grammars (best-effort).
        if let Some(ref dir) = grammar_dir {
            if dir.is_dir() {
                // Future: iterate .so/.dylib files and load via libloading.
                // For now we only use built-in grammars.
                let _ = dir; // suppress unused warning
            }
            // If the directory doesn't exist, we just skip — no error.
        }

        // Register built-in grammars. Each `LanguageFn.into()` yields a
        // `tree_sitter::Language`.
        languages.insert(Language::Rust, tree_sitter_rust::LANGUAGE.into());
        languages.insert(Language::Python, tree_sitter_python::LANGUAGE.into());
        languages.insert(
            Language::TypeScript,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        );
        languages.insert(Language::Go, tree_sitter_go::LANGUAGE.into());

        Ok(Chunker { languages })
    }

    /// Chunk a file into symbol-level pieces.
    pub fn chunk_file(&self, path: &Path, content: &str, language: Language) -> Vec<CodeChunk> {
        if content.is_empty() {
            return Vec::new();
        }

        let ts_lang = match language {
            Language::Unknown => return line_based_fallback(path, content),
            _ => match self.languages.get(&language) {
                Some(l) => l,
                None => return line_based_fallback(path, content),
            },
        };

        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(ts_lang).is_err() {
            return line_based_fallback(path, content);
        }

        let tree = match parser.parse(content, None) {
            Some(t) => t,
            None => return line_based_fallback(path, content),
        };

        let file_hash = sha256_hex(content);
        let lang_str = language_str(language);

        let target_kinds = target_node_kinds(language);
        let mut chunks = Vec::new();

        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            collect_chunks(
                child,
                language,
                &target_kinds,
                content,
                path,
                lang_str,
                &file_hash,
                &mut chunks,
            );
        }

        if chunks.is_empty() {
            return line_based_fallback(path, content);
        }

        chunks
    }

    /// Chunk a text snippet (no file path).
    pub fn chunk_text(&self, content: &str, language: Language) -> Vec<CodeChunk> {
        self.chunk_file(Path::new("<text>"), content, language)
    }

    /// Return the languages currently loaded, sorted.
    pub fn available_languages(&self) -> Vec<Language> {
        let mut langs: Vec<Language> = self.languages.keys().copied().collect();
        langs.sort();
        langs
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Node kinds we extract as top-level chunks for each language.
fn target_node_kinds(language: Language) -> Vec<&'static str> {
    match language {
        Language::Rust => vec![
            "function_item",
            "struct_item",
            "enum_item",
            "impl_item",
            "trait_item",
            "mod_item",
            "macro_definition",
        ],
        Language::Python => vec![
            "function_definition",
            "class_definition",
            "decorated_definition",
        ],
        Language::TypeScript => vec![
            "function_declaration",
            "class_declaration",
            "interface_declaration",
            "type_alias_declaration",
        ],
        Language::Go => vec!["function_declaration", "method_declaration"],
        Language::Unknown => vec![],
    }
}

/// Recursively collect chunks from tree-sitter nodes.
///
/// For Python, when we encounter a `class_definition` we do NOT emit the
/// whole class as a single chunk. Instead we walk into the class body and
/// emit each method (`function_definition`) individually.
fn collect_chunks(
    node: tree_sitter::Node<'_>,
    language: Language,
    target_kinds: &[&str],
    content: &str,
    path: &Path,
    lang_str: &str,
    file_hash: &str,
    chunks: &mut Vec<CodeChunk>,
) {
    let kind = node.kind();

    // Python special handling: walk into class bodies to extract methods.
    if language == Language::Python && kind == "class_definition" {
        extract_python_class_methods(node, content, path, lang_str, file_hash, chunks);
        return;
    }

    if target_kinds.contains(&kind) {
        if let Ok(text) = node.utf8_text(content.as_bytes()) {
            let chunk = make_chunk(node, text, path, lang_str, file_hash);
            chunks.push(chunk);
        }
    }
}

/// For a Python `class_definition` node, walk into the body and emit each
/// `function_definition` (method) as its own chunk.
fn extract_python_class_methods(
    class_node: tree_sitter::Node<'_>,
    content: &str,
    path: &Path,
    lang_str: &str,
    file_hash: &str,
    chunks: &mut Vec<CodeChunk>,
) {
    // Find the `block` child (class body).
    let mut cursor = class_node.walk();
    for child in class_node.children(&mut cursor) {
        if child.kind() == "block" {
            let mut body_cursor = child.walk();
            for body_child in child.children(&mut body_cursor) {
                if body_child.kind() == "function_definition" {
                    if let Ok(text) = body_child.utf8_text(content.as_bytes()) {
                        chunks.push(make_chunk(body_child, text, path, lang_str, file_hash));
                    }
                }
            }
        }
    }
}

/// Create a `CodeChunk` from a tree-sitter node.
fn make_chunk(
    node: tree_sitter::Node<'_>,
    text: &str,
    path: &Path,
    lang_str: &str,
    file_hash: &str,
) -> CodeChunk {
    CodeChunk {
        metadata: CodeMetadata {
            file_path: path.to_path_buf(),
            language: lang_str.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            chunk_content: text.to_string(),
            file_hash: file_hash.to_string(),
        },
        embedding: Vec::new(),
    }
}

/// Language enum to lowercase string.
fn language_str(language: Language) -> &'static str {
    match language {
        Language::Rust => "rust",
        Language::Python => "python",
        Language::TypeScript => "typescript",
        Language::Go => "go",
        Language::Unknown => "unknown",
    }
}

/// Compute SHA-256 of content, returned as a lowercase hex string.
fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

// ---------------------------------------------------------------------------
// Line-based fallback chunker
// ---------------------------------------------------------------------------

/// Split content into chunks at blank lines, capping each chunk at 100 lines.
fn line_based_fallback(path: &Path, content: &str) -> Vec<CodeChunk> {
    if content.is_empty() {
        return Vec::new();
    }

    let file_hash = sha256_hex(content);
    let lines: Vec<&str> = content.lines().collect();

    // Group consecutive non-blank lines.
    let mut groups: Vec<(usize, usize)> = Vec::new(); // (start_idx, end_idx) inclusive, 0-based
    let mut group_start: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            if let Some(start) = group_start.take() {
                groups.push((start, i - 1));
            }
        } else if group_start.is_none() {
            group_start = Some(i);
        }
    }
    // Close final group.
    if let Some(start) = group_start {
        groups.push((start, lines.len() - 1));
    }

    // Sub-split any group exceeding 100 lines.
    let mut chunks = Vec::new();
    for (start, end) in groups {
        let group_len = end - start + 1;
        if group_len <= 100 {
            let text = lines[start..=end].join("\n");
            chunks.push(CodeChunk {
                metadata: CodeMetadata {
                    file_path: path.to_path_buf(),
                    language: "unknown".to_string(),
                    line_start: start + 1,
                    line_end: end + 1,
                    chunk_content: text,
                    file_hash: file_hash.clone(),
                },
                embedding: Vec::new(),
            });
        } else {
            let mut pos = start;
            while pos <= end {
                let sub_end = (pos + 99).min(end);
                let text = lines[pos..=sub_end].join("\n");
                chunks.push(CodeChunk {
                    metadata: CodeMetadata {
                        file_path: path.to_path_buf(),
                        language: "unknown".to_string(),
                        line_start: pos + 1,
                        line_end: sub_end + 1,
                        chunk_content: text,
                        file_hash: file_hash.clone(),
                    },
                    embedding: Vec::new(),
                });
                pos = sub_end + 1;
            }
        }
    }

    chunks
}
