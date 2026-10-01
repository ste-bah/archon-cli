//! The Rust source facts a check-source pin reads (PLAN-11): the test
//! functions a file defines, the module declarations that make a file or a
//! function part of its crate, the file's inner `#![cfg]` attributes, the
//! module files a file declares (`mod x;`, `#[path = ".."] mod x;`), and
//! splicing one such item in or out of a file.
//!
//! A unit test lives inside an implementation file the implementing task must
//! change, so pinning that whole file would forbid the implementation itself.
//! Such a test is pinned as ITEMS, each keyed by where it sits rather than by
//! an index, so adding a same-named test elsewhere cannot shift it:
//!
//! - `fn:<inline modules>::<name>`: the function and the attributes directly
//!   above it (`#[test]`, `#[ignore]`, `#[should_panic]`, ...);
//! - `mod:<inline modules>::<name>`: a module declaration and its
//!   attributes (`#[cfg(test)]`, `#[path]`) -- the whole `mod x;` line, or an
//!   inline module's header up to its `{`, so a module on the way to a
//!   pinned test cannot be switched off or pointed elsewhere;
//! - `cfg:`: the file's inner `#![cfg(..)]` / `#![cfg_attr(..)]` attributes;
//! - `toml:test:<name>`: a manifest's `[[test]]` entry
//!   (`check_source_manifest`).
//!
//! Everything else in the file stays the task's to change.

use std::path::Path;

use tree_sitter::{Node, Parser};

/// The key of a file's inner cfg attributes.
pub const CFG_KEY: &str = "cfg:";

/// One keyed item: its span in the file, attributes included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub key: String,
    pub name: String,
    pub is_test: bool,
    pub start: usize,
    pub end: usize,
}

fn parse(text: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    parser.parse(text, None)
}

/// Every `fn` and `mod` item in `text`, in file order, at any depth, keyed
/// by the inline modules around it; a repeated key gets `#n` (n >= 1).
pub fn items(text: &str) -> Vec<Item> {
    let Some(tree) = parse(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk(tree.root_node(), text, &mut Vec::new(), &mut out);
    out.sort_by_key(|item| item.start);
    let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
    for item in &mut out {
        let n = seen.entry(item.key.clone()).or_default();
        if *n > 0 {
            item.key = format!("{}#{n}", item.key);
        }
        *n += 1;
    }
    out
}

fn walk(node: Node, text: &str, path: &mut Vec<String>, out: &mut Vec<Item>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let kind = child.kind();
        let Some(name) = child
            .child_by_field_name("name")
            .filter(|_| kind == "function_item" || kind == "mod_item")
        else {
            walk(child, text, path, out);
            continue;
        };
        let name = text[name.byte_range()].to_string();
        let (start, attributes) = leading_attributes(child, text);
        let qualified = path
            .iter()
            .cloned()
            .chain(std::iter::once(name.clone()))
            .collect::<Vec<_>>()
            .join("::");
        if kind == "function_item" {
            out.push(Item {
                key: format!("fn:{qualified}"),
                name,
                is_test: attributes.iter().any(|attr| names_test(attr)),
                start,
                end: child.end_byte(),
            });
            continue;
        }
        let body = child.child_by_field_name("body");
        out.push(Item {
            key: format!("mod:{qualified}"),
            name: name.clone(),
            is_test: false,
            start,
            end: body.map_or(child.end_byte(), |body| body.start_byte()),
        });
        if let Some(body) = body {
            path.push(name);
            walk(body, text, path, out);
            path.pop();
        }
    }
}

/// The attribute items directly above `node` (no other item between) and
/// where the first of them starts.
fn leading_attributes<'t>(node: Node, text: &'t str) -> (usize, Vec<&'t str>) {
    let mut start = node.start_byte();
    let mut attributes = Vec::new();
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        match sibling.kind() {
            "attribute_item" => {
                attributes.push(&text[sibling.byte_range()]);
                start = sibling.start_byte();
            }
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        previous = sibling.prev_named_sibling();
    }
    (start, attributes)
}

/// `#[test]`, `#[tokio::test]`, `#[rstest]`, `#[test_case(..)]`, ...
fn names_test(attribute: &str) -> bool {
    attribute
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == "test" || word.starts_with("test_") || word == "rstest")
}

/// The name a key names: its last path segment, without a `#n`.
pub fn key_name(key: &str) -> &str {
    let bare = key.split_once(':').map_or(key, |(_, rest)| rest);
    let bare = bare.rsplit_once('#').map_or(bare, |(name, _)| name);
    bare.rsplit("::").next().unwrap_or(bare)
}

/// The `mod:` keys of the inline modules around `key` in its file,
/// outermost first.
pub fn enclosing_mods(key: &str) -> Vec<String> {
    let Some((_, rest)) = key.split_once(':') else {
        return Vec::new();
    };
    let rest = rest.rsplit_once('#').map_or(rest, |(path, _)| path);
    let parts: Vec<&str> = rest.split("::").collect();
    (1..parts.len())
        .map(|len| format!("mod:{}", parts[..len].join("::")))
        .collect()
}

/// Every test function in `text`.
pub fn keyed_tests(text: &str) -> Vec<Item> {
    items(text)
        .into_iter()
        .filter(|item| item.is_test && item.key.starts_with("fn:"))
        .collect()
}

/// The test functions a name filter selects: `exact` compares the whole
/// name, otherwise a name containing `filter` matches (cargo's substring
/// rule, applied to the function name).
pub fn matching_tests(text: &str, filter: &str, exact: bool) -> Vec<Item> {
    keyed_tests(text)
        .into_iter()
        .filter(|item| {
            if exact {
                item.name == filter
            } else {
                item.name.contains(filter)
            }
        })
        .collect()
}

/// Inner `#![cfg..]` attributes at the top of the file, as spans.
fn inner_cfgs(text: &str) -> Vec<(usize, usize)> {
    let Some(tree) = parse(text) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .filter(|node| node.kind() == "inner_attribute_item")
        .filter(|node| {
            let attr = text[node.byte_range()]
                .trim_start_matches("#![")
                .trim_start();
            attr.starts_with("cfg")
        })
        .map(|node| (node.start_byte(), node.end_byte()))
        .collect()
}

/// The text of the item keyed `key`, if `text` has it. `cfg:` is always
/// present: the file's inner cfg attributes, one per line, possibly none.
pub fn item_text(text: &str, key: &str) -> Option<String> {
    if key.starts_with("toml:") {
        return crate::check_source_manifest::entry_text(text, key);
    }
    if key == CFG_KEY {
        return Some(
            inner_cfgs(text)
                .into_iter()
                .map(|(start, end)| &text[start..end])
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    items(text)
        .into_iter()
        .find(|item| item.key == key)
        .map(|item| text[item.start..item.end].to_string())
}

/// `text` with the item keyed `key` replaced by `replacement`, or removed
/// (with the line break after it) when `replacement` is `None`. `None` when
/// `text` has no such item to replace.
pub fn splice_item(text: &str, key: &str, replacement: Option<&str>) -> Option<String> {
    if key.starts_with("toml:") {
        return crate::check_source_manifest::splice_entry(text, key, replacement);
    }
    if key == CFG_KEY {
        let mut out = text.to_string();
        for (start, end) in inner_cfgs(text).into_iter().rev() {
            let end = if out[end..].starts_with('\n') {
                end + 1
            } else {
                end
            };
            out.replace_range(start..end, "");
        }
        return Some(match replacement.filter(|r| !r.is_empty()) {
            Some(cfgs) => format!("{cfgs}\n{out}"),
            None => out,
        });
    }
    let item = items(text).into_iter().find(|item| item.key == key)?;
    let (mut start, mut end) = (item.start, item.end);
    if replacement.is_none() {
        if text[end..].starts_with('\n') {
            end += 1;
        }
        // The removed item's own indentation goes with it.
        let line_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
        if text[line_start..start].trim().is_empty() {
            start = line_start;
        }
    }
    Some(format!(
        "{}{}{}",
        &text[..start],
        replacement.unwrap_or_default(),
        &text[end..]
    ))
}

/// One module file a file loads, and the `mod:` key declaring it there.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModChild {
    pub path: String,
    pub decl: String,
    /// The path leaves the tree it is relative to (`..` past its root):
    /// refused, never folded into a path inside it.
    pub escapes: bool,
}

/// The files `file`'s out-of-line `mod x;` declarations load, repository-
/// relative: for each, the first of `dir/x.rs` and `dir/x/mod.rs` that
/// exists under `root`, else `dir/x.rs` (not created yet). `crate_root`
/// (a `lib.rs`, `main.rs`, `mod.rs` or an integration test's top file)
/// owns its own directory; any other file owns `dir/<stem>/`. A
/// `#[path]` attribute is relative to the file's own directory.
pub fn mod_children(root: &Path, file: &str, text: &str, crate_root: bool) -> Vec<ModChild> {
    let Some(tree) = parse(text) else {
        return Vec::new();
    };
    let path = Path::new(file);
    let parent = path.parent().unwrap_or(Path::new("")).to_path_buf();
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let owns_parent = crate_root || matches!(stem, "mod" | "lib" | "main");
    let own_dir = if owns_parent {
        parent.clone()
    } else {
        parent.join(stem)
    };
    let mut out = Vec::new();
    let site = Site {
        text,
        file_dir: &parent,
        root,
    };
    collect_mods(tree.root_node(), &site, &own_dir, &mut Vec::new(), &mut out);
    out.sort();
    out.dedup();
    out
}

struct Site<'a> {
    text: &'a str,
    file_dir: &'a Path,
    root: &'a Path,
}

fn collect_mods(
    node: Node,
    site: &Site,
    dir: &Path,
    inline: &mut Vec<String>,
    out: &mut Vec<ModChild>,
) {
    let text = site.text;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "mod_item" {
            continue;
        }
        let Some(name) = child.child_by_field_name("name") else {
            continue;
        };
        let name = text[name.byte_range()].to_string();
        let (_, attributes) = leading_attributes(child, text);
        let explicit = attributes.iter().find_map(|attr| path_attribute(attr));
        match child.child_by_field_name("body") {
            Some(body) => {
                // An inline module nests the directory its children load from.
                let nested = match &explicit {
                    Some(explicit) => site.file_dir.join(explicit),
                    None => dir.join(&name),
                };
                inline.push(name);
                collect_mods(body, site, &nested, inline, out);
                inline.pop();
            }
            None => {
                let candidates = match &explicit {
                    Some(explicit) => vec![site.file_dir.join(explicit)],
                    None => vec![
                        dir.join(format!("{name}.rs")),
                        dir.join(&name).join("mod.rs"),
                    ],
                };
                let chosen = candidates
                    .iter()
                    .find(|candidate| site.root.join(candidate).is_file())
                    .unwrap_or(&candidates[0]);
                let qualified = inline
                    .iter()
                    .cloned()
                    .chain(std::iter::once(name))
                    .collect::<Vec<_>>()
                    .join("::");
                let (path, escapes) = match normalized_within(chosen) {
                    Some(path) => (path, false),
                    None => (chosen.display().to_string(), true),
                };
                out.push(ModChild {
                    path,
                    decl: format!("mod:{qualified}"),
                    escapes,
                });
            }
        }
    }
}

/// The value of `#[path = "..."]`.
fn path_attribute(attribute: &str) -> Option<String> {
    let inner = attribute
        .trim()
        .strip_prefix("#[")?
        .strip_suffix(']')?
        .trim();
    let value = inner.strip_prefix("path")?.trim_start().strip_prefix('=')?;
    Some(value.trim().trim_matches('"').to_string())
}

/// `path` as a clean forward-slash relative path, `..` folded, or `None`
/// when a `..` would climb above where it starts.
pub fn normalized_within(path: &Path) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                parts.pop()?;
            }
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// `path` as a clean forward-slash relative path, `..` folded.
pub fn normalized(path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    parts.join("/")
}

#[cfg(test)]
#[path = "check_source_rust_tests.rs"]
mod tests;
