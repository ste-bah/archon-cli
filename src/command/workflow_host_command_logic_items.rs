//! Issue 361 (tests only): the module-level items of one Rust source file,
//! each with the names it defines and the paths, names and macros its own
//! tokens use. The logic closure follows a reference into the items it
//! names, not into every item of the file that holds them.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::workflow_host_command_logic_source::{
    KEYWORDS, Tok, attribute, impl_type, include_target, is, lex, skip_item, skip_statement,
    test_cfg, use_tree, word,
};

/// One `use` leaf: the path it names, and the name it binds (`None` for a
/// glob).
#[derive(Clone, Debug)]
pub(crate) struct UseDecl {
    pub(crate) scope: Vec<String>,
    pub(crate) path: Vec<String>,
    pub(crate) alias: Option<String>,
    pub(crate) public: bool,
}

/// One module-level item and what it references.
#[derive(Clone, Debug, Default)]
pub(crate) struct Item {
    /// The inline module it is in (empty for the file's own module).
    pub(crate) scope: Vec<String>,
    pub(crate) names: BTreeSet<String>,
    /// The type an `impl` block is for.
    pub(crate) impl_for: Option<String>,
    /// An item-level macro call (`thread_local! { … }`): the names it
    /// defines are read from inside it.
    pub(crate) macro_call: bool,
    pub(crate) paths: BTreeSet<Vec<String>>,
    pub(crate) idents: BTreeSet<String>,
    pub(crate) invoked: BTreeSet<String>,
}

impl Item {
    /// No name reaches it (an item-level macro call, an extern block, a
    /// `const _`), so it is reached with its file.
    pub(crate) fn anonymous(&self) -> bool {
        self.names.is_empty() && self.impl_for.is_none()
    }
}

/// What a file declares and uses. Items behind `#[cfg(test)]` are left out:
/// tests never stand for a verdict.
#[derive(Debug, Default)]
pub(crate) struct Facts {
    pub(crate) test_only: bool,
    /// Out-of-line modules: scope, name, `#[path]`.
    pub(crate) mods: Vec<(Vec<String>, String, Option<String>)>,
    pub(crate) inline: BTreeSet<Vec<String>>,
    pub(crate) items: Vec<Item>,
    pub(crate) uses: Vec<UseDecl>,
    /// `include!`, `include_str!` and `include_bytes!` targets: whether the
    /// target is Rust source spliced into this module, whether its path is
    /// relative to the crate manifest, and the path.
    pub(crate) includes: Vec<(bool, bool, String)>,
    pub(crate) unparsed: Vec<String>,
}

impl Facts {
    /// The items of `scope` that define `name`.
    pub(crate) fn defining(&self, scope: &[String], name: &str) -> Vec<usize> {
        let items = self.items.iter().enumerate();
        items
            .filter(|(_, item)| item.scope == scope && item.names.contains(name))
            .map(|(at, _)| at)
            .collect()
    }

    /// Whether any item of this file names `name` (outside `use`).
    pub(crate) fn mentions(&self, name: &str) -> bool {
        self.items.iter().any(|item| {
            item.idents.contains(name)
                || item.invoked.contains(name)
                || item
                    .paths
                    .iter()
                    .any(|path| path.first().map(String::as_str) == Some(name))
        })
    }

    /// Splices the facts of a file `include!`d at this file's top level.
    pub(crate) fn splice(&mut self, other: Facts) {
        self.mods.extend(other.mods);
        self.inline.extend(other.inline);
        self.items.extend(other.items);
        self.uses.extend(other.uses);
        self.includes.extend(other.includes);
        self.unparsed.extend(other.unparsed);
    }
}

/// Everything [`Facts`] holds for the source `text`.
pub(crate) fn facts(text: &str) -> Facts {
    let (toks, _) = lex(text);
    let mut facts = Facts::default();
    let (mut i, mut depth, mut nest) = (0usize, 0usize, 0i32);
    let mut scopes: Vec<(String, usize)> = Vec::new();
    let (mut path_attr, mut test, mut public, mut ended) = (None::<String>, false, false, true);
    while i < toks.len() {
        let level = scopes.last().map_or(0, |(_, body)| *body);
        let scope = scopes
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        let item_level = depth == level && nest == 0;
        let tok = &toks[i];
        if item_level && ended && *tok != Tok::Punct('}') {
            facts.items.push(Item {
                scope: scope.clone(),
                ..Item::default()
            });
            ended = false;
        }
        let cur = facts.items.len().checked_sub(1);
        if *tok == Tok::Punct('#') && (is(toks.get(i + 1), '[') || is(toks.get(i + 1), '!')) {
            let inner = is(toks.get(i + 1), '!');
            let (attr, after) = attribute(&toks, i);
            if let [Tok::Ident(p), Tok::Punct('='), Tok::Str(path)] = attr
                && p == "path"
                && item_level
            {
                path_attr = Some(path.clone());
            }
            if test_cfg(attr) && !inner && !item_level {
                i = skip_statement(&toks, after);
                continue;
            }
            if test_cfg(attr) {
                facts.test_only |= inner && depth == 0;
                test |= !inner && item_level;
            }
            i = after;
            continue;
        }
        if item_level && test {
            facts.items.pop();
            (i, test, path_attr, public, ended) = (skip_item(&toks, i), false, None, false, true);
            continue;
        }
        match tok {
            Tok::Punct('{') => depth += 1,
            Tok::Punct('}') => {
                depth = depth.saturating_sub(1);
                if depth < level {
                    scopes.pop();
                }
                ended |= depth <= level && nest == 0;
            }
            Tok::Punct('(' | '[') => nest += 1,
            Tok::Punct(')' | ']') => nest -= 1,
            Tok::Punct(';') if item_level => ended = true,
            _ => {}
        }
        let w = word(Some(tok));
        let defines = matches!(
            w,
            Some("fn" | "struct" | "enum" | "trait" | "type" | "const" | "static" | "union")
        );
        if let Some(cur) = cur.filter(|cur| !item_level && facts.items[*cur].macro_call)
            && defines
            && let Some(name) = word(toks.get(i + 1)).filter(|n| !KEYWORDS.contains(n))
        {
            facts.items[cur].names.insert(name.into());
        }
        if item_level
            && is(toks.get(i + 1), '!')
            && let (Some(cur), Some(name)) = (cur, w)
        {
            facts.items[cur].macro_call = name != "macro_rules";
        }
        if item_level {
            match w {
                Some("pub") => {
                    public = true;
                    i += 1;
                    if is(toks.get(i), '(') {
                        let close = toks[i..].iter().position(|t| *t == Tok::Punct(')'));
                        i += close.map_or(0, |close| close + 1);
                    }
                    continue;
                }
                Some("mod") => match (word(toks.get(i + 1)), toks.get(i + 2)) {
                    (Some(name), Some(Tok::Punct(';'))) => {
                        facts.mods.push((scope, name.into(), path_attr.take()));
                        (i, public, ended) = (i + 3, false, true);
                        continue;
                    }
                    (Some(name), Some(Tok::Punct('{'))) => {
                        facts
                            .inline
                            .insert([scope, vec![name.to_string()]].concat());
                        depth += 1;
                        scopes.push((name.into(), depth));
                        (i, public, path_attr, ended) = (i + 3, false, None, true);
                        continue;
                    }
                    _ => facts.unparsed.push(format!("mod at token {i}")),
                },
                Some(
                    "fn" | "struct" | "enum" | "trait" | "type" | "const" | "static" | "union",
                ) => {
                    if let (Some(name), Some(cur)) =
                        (word(toks.get(i + 1)).filter(|n| !KEYWORDS.contains(n)), cur)
                    {
                        facts.items[cur].names.insert(name.into());
                    }
                }
                Some("macro_rules") if is(toks.get(i + 1), '!') => {
                    if let (Some(name), Some(cur)) = (word(toks.get(i + 2)), cur) {
                        facts.items[cur].names.insert(name.into());
                    }
                }
                Some("impl") => {
                    if let Some(cur) = cur.filter(|cur| facts.items[*cur].names.is_empty()) {
                        facts.items[cur].impl_for = impl_type(&toks, i + 1);
                    }
                }
                _ => {}
            }
            if !matches!(
                w,
                Some("unsafe" | "async" | "extern" | "const" | "default" | "use")
            ) {
                (public, path_attr) = (false, None);
            }
        }
        if w == Some("use") {
            let mut leaves = Vec::new();
            let end = use_tree(&toks, i + 1, Vec::new(), &mut leaves);
            for (path, alias) in leaves {
                if !item_level && let (Some(cur), Some(_)) = (cur, &alias) {
                    facts.items[cur].paths.insert(path.clone());
                }
                let public = public && item_level;
                facts.uses.push(UseDecl {
                    scope: scope.clone(),
                    path,
                    alias,
                    public,
                });
            }
            (i, public, path_attr) = (end, false, None);
            ended |= item_level;
            continue;
        }
        if let Some(first) = w {
            let leading = toks.get(i.wrapping_sub(1)) == Some(&Tok::PathSep);
            let qualified = leading
                && matches!(
                    toks.get(i.wrapping_sub(2)),
                    Some(Tok::Ident(_)) | Some(Tok::Punct('>'))
                );
            if !qualified && !is(toks.get(i.wrapping_sub(1)), '.') {
                let mut segs = if leading {
                    vec![String::new()]
                } else {
                    Vec::new()
                };
                segs.push(first.to_string());
                let mut at = i + 1;
                while toks.get(at) == Some(&Tok::PathSep)
                    && let Some(seg) = word(toks.get(at + 1))
                {
                    segs.push(seg.into());
                    at += 2;
                }
                let bang = is(toks.get(at), '!');
                if matches!(first, "include_str" | "include_bytes" | "include") && bang {
                    match include_target(&toks, at + 2) {
                        Some((manifest, path)) => {
                            facts.includes.push((first == "include", manifest, path));
                        }
                        None => facts
                            .unparsed
                            .push(format!("{first}! with a computed path")),
                    }
                } else if let Some(cur) = cur {
                    let item = &mut facts.items[cur];
                    if segs.len() > 1 {
                        item.paths.insert(segs);
                    } else if bang {
                        item.invoked.insert(first.into());
                    } else if !KEYWORDS.contains(&first) {
                        item.idents.insert(first.into());
                    }
                }
                i = at;
                continue;
            }
        }
        i += 1;
    }
    facts
}

/// Test modules never stand for a capability's logic.
pub(crate) fn is_test_file(file: &Path) -> bool {
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    file.components().any(|part| part.as_os_str() == "tests")
        || stem == "tests"
        || stem.ends_with("_tests")
        || stem.contains("_tests_")
        || stem.ends_with("_test")
        || stem.contains("test_support")
        || stem.contains("fixture")
}

/// The file of module `name`, declared in `scope` of `file`.
pub(crate) fn child_file(file: &Path, scope: &[String], name: &str, attr: Option<&str>) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("")).to_path_buf();
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let owned = if matches!(stem, "mod" | "lib" | "main") {
        dir.clone()
    } else {
        dir.join(stem)
    };
    let nested = |base: &Path| {
        scope
            .iter()
            .fold(base.to_path_buf(), |at, seg| at.join(seg))
    };
    if let Some(attr) = attr {
        return if scope.is_empty() {
            dir.join(attr)
        } else {
            nested(&owned).join(attr)
        };
    }
    // A file loaded by `#[path]` owns its directory like a `mod.rs`.
    [nested(&owned), nested(&dir)]
        .into_iter()
        .flat_map(|at| [at.join(format!("{name}.rs")), at.join(name).join("mod.rs")])
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| nested(&owned).join(format!("{name}.rs")))
}

/// Every workspace crate, by the name code uses for it: its root file and
/// manifest directory. `binary` names the crate of `src/main.rs`.
pub(crate) fn workspace_crates(
    base: &Path,
    binary: &str,
) -> std::collections::BTreeMap<String, (PathBuf, PathBuf)> {
    let mut roots = std::collections::BTreeMap::from([(
        binary.to_string(),
        (base.join("src/main.rs"), base.to_path_buf()),
    )]);
    let members = std::fs::read_dir(base.join("crates")).into_iter().flatten();
    for dir in members.flatten().map(|entry| entry.path()) {
        let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
            continue;
        };
        let name = manifest
            .lines()
            .skip_while(|line| line.trim() != "[package]")
            .find_map(|line| line.trim().strip_prefix("name = \""))
            .and_then(|rest| rest.split('"').next());
        if let Some(name) = name
            && dir.join("src/lib.rs").exists()
        {
            roots.insert(
                name.replace('-', "_"),
                (dir.join("src/lib.rs"), dir.clone()),
            );
        }
    }
    roots
}
