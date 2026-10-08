//! Issue 361 (tests only): the source files a capability's verdict is built
//! from.
//!
//! The walk starts at the capability's root modules and follows their `mod`
//! declarations (each root's whole module subtree). From every item it
//! reaches it follows each `use`, path, name and macro that resolves into a
//! workspace crate, to the items those name (and to the `impl` blocks of a
//! type it names), and every file that holds a reached item is hashed whole,
//! with the files it `include!`s. A dependency a later change adds is
//! covered without anyone listing it; only a reviewed denylist entry leaves
//! a reached file out.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use super::workflow_host_command_logic_items::{
    Facts, child_file, facts, is_test_file, workspace_crates,
};

/// An item: its crate, its file, its index in the file's facts.
type ItemRef = (String, PathBuf, usize);

/// One crate's module tree.
#[derive(Default)]
struct Crate {
    /// Module path to the file and the inline scope inside it.
    modules: BTreeMap<Vec<String>, (PathBuf, Vec<String>)>,
    files: BTreeMap<PathBuf, Vec<String>>,
    /// Type name to the `impl` blocks for it.
    impls: BTreeMap<String, BTreeSet<(PathBuf, usize)>>,
}

/// What a path or name resolves to.
#[derive(Clone)]
enum Res {
    Module(String, Vec<String>),
    Items(BTreeSet<ItemRef>),
    External,
}

/// The hashed files of a closure, and the denylist entries it stopped at.
#[derive(Debug, Default)]
pub(crate) struct Closure {
    pub(crate) files: BTreeSet<PathBuf>,
    pub(crate) denied: BTreeMap<String, BTreeSet<PathBuf>>,
    /// Each hashed file's references to other workspace files, denied and
    /// dropped ones included.
    pub(crate) references: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    /// The file through which each reached file was first reached.
    pub(crate) parents: BTreeMap<PathBuf, PathBuf>,
    /// Every resolved reference, item to item (`usize::MAX` for the file
    /// itself). It is in the digest, so re-pointing a `use` between two
    /// hashed items moves it even when no hashed file changed.
    pub(crate) edges: BTreeSet<(PathBuf, usize, PathBuf, usize)>,
}

pub(crate) struct Workspace {
    base: PathBuf,
    /// Crate name (as code names it) to its root file and manifest directory.
    roots: BTreeMap<String, (PathBuf, PathBuf)>,
    crates: BTreeMap<String, Crate>,
    facts: BTreeMap<PathBuf, Facts>,
    /// Resolved names; `None` while one is being resolved (a cycle).
    names: BTreeMap<(String, Vec<String>, String), Option<Res>>,
    /// References the walk could not resolve; a sound closure has none.
    pub(crate) errors: BTreeSet<String>,
}

/// The binary crate the host-command subcommands are built into.
const BINARY: &str = "archon";

impl Workspace {
    pub(crate) fn new(base: &Path) -> Self {
        let roots = workspace_crates(base, BINARY);
        Self {
            base: base.to_path_buf(),
            roots,
            crates: BTreeMap::new(),
            facts: BTreeMap::new(),
            names: BTreeMap::new(),
            errors: BTreeSet::new(),
        }
    }

    /// `file` under the base, with `.` and `..` folded away.
    fn rel(&self, file: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for part in file.strip_prefix(&self.base).unwrap_or(file).components() {
            match part {
                std::path::Component::ParentDir => {
                    out.pop();
                }
                std::path::Component::CurDir => {}
                other => out.push(other),
            }
        }
        out
    }

    /// A file's facts, with every Rust file it `include!`s spliced in (the
    /// spliced files stay among its include targets, so they are hashed).
    fn facts(&mut self, file: &Path) -> &Facts {
        if !self.facts.contains_key(file) {
            let mut parsed = self.parse(file);
            let mut spliced = BTreeSet::new();
            while let Some(path) = parsed
                .includes
                .iter()
                .find(|(code, manifest, path)| *code && !*manifest && !spliced.contains(path))
                .map(|(_, _, path)| path.clone())
            {
                let other = self.parse(Path::new(&path));
                parsed.splice(other);
                spliced.insert(path);
            }
            self.facts.insert(file.to_path_buf(), parsed);
        }
        &self.facts[file]
    }

    /// One file's own facts, its relative include paths made absolute.
    fn parse(&mut self, file: &Path) -> Facts {
        let text = std::fs::read_to_string(file).unwrap_or_else(|error| {
            self.errors
                .insert(format!("{} unreadable: {error}", file.display()));
            String::new()
        });
        let mut parsed = facts(&text);
        let dir = file.parent().unwrap_or(Path::new(""));
        for (_, manifest, path) in &mut parsed.includes {
            if !*manifest {
                *path = dir.join(&*path).to_string_lossy().into_owned();
            }
        }
        // So a `#[path]` in a spliced file stays relative to that file.
        for (scope, _, attr) in &mut parsed.mods {
            if let Some(attr) = attr.as_mut().filter(|_| scope.is_empty()) {
                *attr = dir.join(&*attr).to_string_lossy().into_owned();
            }
        }
        parsed
    }

    fn load(&mut self, krate: &str) -> bool {
        if self.crates.contains_key(krate) {
            return true;
        }
        let Some((root, _)) = self.roots.get(krate).cloned() else {
            return false;
        };
        let mut tree = Crate::default();
        let mut stack = vec![(root, Vec::<String>::new())];
        while let Some((file, path)) = stack.pop() {
            if tree.files.contains_key(&file) || is_test_file(&self.rel(&file)) {
                continue;
            }
            if !file.exists() {
                let missing = self.rel(&file);
                self.errors
                    .insert(format!("module file {} is missing", missing.display()));
                continue;
            }
            let facts = self.facts(&file);
            if facts.test_only {
                continue;
            }
            let mods = facts.mods.clone();
            tree.files.insert(file.clone(), path.clone());
            tree.modules
                .insert(path.clone(), (file.clone(), Vec::new()));
            for scope in &facts.inline {
                let module = [path.clone(), scope.clone()].concat();
                tree.modules.insert(module, (file.clone(), scope.clone()));
            }
            for (at, item) in facts.items.iter().enumerate() {
                if let Some(name) = &item.impl_for {
                    tree.impls
                        .entry(name.clone())
                        .or_default()
                        .insert((file.clone(), at));
                }
            }
            for (scope, name, attr) in mods {
                let child = child_file(&file, &scope, &name, attr.as_deref());
                stack.push((child, [path.clone(), scope, vec![name]].concat()));
            }
        }
        self.crates.insert(krate.to_string(), tree);
        true
    }

    /// What `name` means inside module `module` of `krate`.
    fn resolve_name(&mut self, krate: &str, module: &[String], name: &str) -> Option<Res> {
        let key = (krate.to_string(), module.to_vec(), name.to_string());
        if let Some(known) = self.names.get(&key) {
            return known.clone();
        }
        self.names.insert(key.clone(), None);
        let res = self.lookup(krate, module, name);
        self.names.insert(key, res.clone());
        res
    }

    fn lookup(&mut self, krate: &str, module: &[String], name: &str) -> Option<Res> {
        if !self.load(krate) {
            return None;
        }
        let tree = &self.crates[krate];
        let child = [module, &[name.to_string()]].concat();
        if tree.modules.contains_key(&child) {
            return Some(Res::Module(krate.to_string(), child));
        }
        let (file, scope) = tree.modules.get(module)?.clone();
        let impls = tree.impls.get(name).cloned().unwrap_or_default();
        let facts = self.facts(&file);
        let defining = facts.defining(&scope, name);
        if !defining.is_empty() {
            let own = defining.into_iter().map(|at| (file.clone(), at));
            let items = own
                .chain(impls)
                .map(|(at_file, at)| (krate.to_string(), at_file, at));
            return Some(Res::Items(items.collect()));
        }
        let uses = facts
            .uses
            .iter()
            .filter(|u| u.scope == scope)
            .cloned()
            .collect::<Vec<_>>();
        let mut found = None::<Res>;
        for named in uses.iter().filter(|u| u.alias.as_deref() == Some(name)) {
            match (found.take(), self.resolve_path(krate, module, &named.path)) {
                (Some(Res::Items(mut a)), Ok(Res::Items(b))) => {
                    a.extend(b);
                    found = Some(Res::Items(a));
                }
                (Some(Res::Module(k, m)), _) => found = Some(Res::Module(k, m)),
                (_, Ok(res)) => found = Some(res),
                (previous, Err(error)) => {
                    self.errors.insert(error);
                    found = previous;
                }
            }
        }
        if found.is_some() {
            return found;
        }
        for glob in uses.iter().filter(|u| u.alias.is_none()) {
            match self.resolve_path(krate, module, &glob.path) {
                Ok(Res::Module(k, m)) => {
                    if let Some(res) = self.resolve_name(&k, &m, name) {
                        return Some(res);
                    }
                }
                Ok(Res::Items(items)) if name.starts_with(|c: char| c.is_ascii_uppercase()) => {
                    return Some(Res::Items(items));
                }
                Ok(_) => {}
                Err(error) => {
                    self.errors.insert(error);
                }
            }
        }
        None
    }

    /// What the path `segs`, written inside module `module` of `krate`, means.
    fn resolve_path(
        &mut self,
        krate: &str,
        module: &[String],
        segs: &[String],
    ) -> Result<Res, String> {
        let Some(first) = segs.first() else {
            return Ok(Res::External);
        };
        let (target, mut at, mut rest): (String, Vec<String>, &[String]) = match first.as_str() {
            "" => match segs.get(1) {
                Some(name) if self.roots.contains_key(name) => {
                    (name.clone(), Vec::new(), &segs[2..])
                }
                _ => return Ok(Res::External),
            },
            "crate" => (krate.to_string(), Vec::new(), &segs[1..]),
            "self" => (krate.to_string(), module.to_vec(), &segs[1..]),
            "super" => {
                let ups = segs.iter().take_while(|s| *s == "super").count();
                let keep = module.len().saturating_sub(ups);
                (krate.to_string(), module[..keep].to_vec(), &segs[ups..])
            }
            "Self" => return Ok(Res::External),
            name => match self.resolve_name(krate, module, name) {
                Some(Res::Module(k, m)) => (k, m, &segs[1..]),
                Some(res) => return Ok(res),
                None if self.roots.contains_key(name) => (name.to_string(), Vec::new(), &segs[1..]),
                None => return Ok(Res::External),
            },
        };
        while let Some(name) = rest.first() {
            match self.resolve_name(&target, &at, name) {
                Some(Res::Module(_, m)) => at = m,
                Some(res) => return Ok(res),
                None => {
                    let module = at.join("::");
                    return Err(format!(
                        "{target}::{module}::{name} (as {})",
                        segs.join("::")
                    ));
                }
            }
            rest = &rest[1..];
        }
        Ok(Res::Module(target, at))
    }

    fn items_of(&mut self, res: Result<Res, String>, at: &Path) -> Vec<ItemRef> {
        match res {
            Ok(Res::Items(items)) => items.into_iter().collect(),
            Ok(_) => Vec::new(),
            Err(error) => {
                self.errors.insert(format!("{}: {error}", at.display()));
                Vec::new()
            }
        }
    }

    /// The items item `at` of `file` (in `krate`) references.
    fn item_references(&mut self, krate: &str, file: &Path, at: usize) -> Vec<ItemRef> {
        let module = self.crates[krate].files[file].clone();
        let item = self.facts(file).items[at].clone();
        let scope = [module, item.scope.clone()].concat();
        let rel = self.rel(file);
        let mut out = Vec::new();
        for path in &item.paths {
            let res = self.resolve_path(krate, &scope, path);
            out.extend(self.items_of(res, &rel));
        }
        for name in item.idents.iter().chain(&item.invoked) {
            if let Some(res @ Res::Items(_)) = self.resolve_name(krate, &scope, name) {
                out.extend(self.items_of(Ok(res), &rel));
            }
        }
        out
    }

    /// What touching `file` reaches by itself: its anonymous items, the
    /// private imports no item names (trait imports), and its includes.
    fn file_references(&mut self, krate: &str, file: &Path) -> (Vec<ItemRef>, Vec<PathBuf>) {
        let module = self.crates[krate].files[file].clone();
        let rel = self.rel(file);
        let facts = self.facts(file);
        let anonymous = facts
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.anonymous());
        let anonymous = anonymous.map(|(at, _)| (krate.to_string(), file.to_path_buf(), at));
        let mut out = anonymous.collect::<Vec<_>>();
        let traits = facts
            .uses
            .iter()
            .filter(|u| !u.public && u.alias.as_ref().is_some_and(|alias| !facts.mentions(alias)))
            .cloned()
            .collect::<Vec<_>>();
        let (includes, unparsed) = (facts.includes.clone(), facts.unparsed.clone());
        for error in unparsed {
            self.errors.insert(format!("{}: {error}", rel.display()));
        }
        for import in traits {
            let scope = [module.clone(), import.scope].concat();
            let res = self.resolve_path(krate, &scope, &import.path);
            out.extend(self.items_of(res, &rel));
        }
        let mut assets = Vec::new();
        for (_, manifest, path) in includes {
            let target = if manifest {
                self.roots[krate].1.join(path.trim_start_matches('/'))
            } else {
                PathBuf::from(&path)
            };
            if target.exists() {
                assets.push(target);
            } else {
                self.errors
                    .insert(format!("{}: include of missing {path}", rel.display()));
            }
        }
        (out, assets)
    }

    fn denied<'d>(rel: &Path, deny: &[(&'d str, &str)]) -> Option<&'d str> {
        deny.iter()
            .map(|(entry, _)| *entry)
            .find(|entry| rel == Path::new(entry) || entry.ends_with('/') && rel.starts_with(entry))
    }

    fn crate_of(&self, root: &str) -> String {
        match root
            .strip_prefix("crates/")
            .and_then(|r| r.split('/').next())
        {
            Some(dir) => {
                let found = self.roots.iter().find(|(_, (_, at))| at.ends_with(dir));
                found.map_or_else(|| dir.replace('-', "_"), |(name, _)| name.clone())
            }
            None => BINARY.to_string(),
        }
    }

    /// The closure of `roots` (paths under the base), stopping at `deny`
    /// (relative paths; one ending in `/` covers a directory).
    pub(crate) fn closure(&mut self, roots: &[&str], deny: &[(&str, &str)]) -> Closure {
        let mut closure = Closure::default();
        let mut queue = VecDeque::new();
        for root in roots {
            let krate = self.crate_of(root);
            self.load(&krate);
            queue.push_back((krate, self.base.join(root), None::<usize>, None::<PathBuf>));
        }
        let (mut items, mut touched) = (BTreeSet::new(), BTreeSet::new());
        while let Some((krate, file, item, from)) = queue.pop_front() {
            let rel = self.rel(&file);
            // Recorded before any stop, so a reference that lands on a file
            // the walk then drops shows as neither hashed nor denied.
            if let Some(from) = from.filter(|from| *from != rel) {
                closure
                    .references
                    .entry(from.clone())
                    .or_default()
                    .insert(rel.clone());
                closure.parents.entry(rel.clone()).or_insert(from);
            }
            if let Some(entry) = Self::denied(&rel, deny) {
                closure
                    .denied
                    .entry(entry.to_string())
                    .or_default()
                    .insert(rel);
                continue;
            }
            if is_test_file(&rel) || !self.crates[&krate].files.contains_key(&file) {
                continue;
            }
            let mut reached = Vec::new();
            if touched.insert(file.clone()) {
                closure.files.insert(rel.clone());
                let (refs, assets) = self.file_references(&krate, &file);
                reached.extend(refs.into_iter().map(|target| (usize::MAX, target)));
                for asset in assets.iter().map(|asset| self.rel(asset)) {
                    closure
                        .references
                        .entry(rel.clone())
                        .or_default()
                        .insert(asset.clone());
                    closure.parents.entry(asset.clone()).or_insert(rel.clone());
                    closure.files.insert(asset);
                }
            }
            let wanted = match item {
                Some(at) => vec![at],
                None => {
                    // A root: every item, and every module it declares.
                    let module = self.crates[&krate].files[&file].clone();
                    for (scope, name, _) in self.facts(&file).mods.clone() {
                        let child = [module.clone(), scope, vec![name]].concat();
                        if let Some((child, _)) = self.crates[&krate].modules.get(&child) {
                            let next = (krate.clone(), child.clone(), None, Some(rel.clone()));
                            queue.push_back(next);
                        }
                    }
                    (0..self.facts(&file).items.len()).collect()
                }
            };
            for at in wanted {
                if items.insert((file.clone(), at)) {
                    let refs = self.item_references(&krate, &file, at);
                    reached.extend(refs.into_iter().map(|target| (at, target)));
                }
            }
            for (source, (k, f, at)) in reached {
                let target = self.rel(&f);
                if Self::denied(&target, deny).is_none() && !is_test_file(&target) {
                    closure
                        .edges
                        .insert((rel.clone(), source, self.rel(&f), at));
                }
                if !items.contains(&(f.clone(), at)) {
                    queue.push_back((k, f, Some(at), Some(rel.clone())));
                }
            }
        }
        closure
    }
}
