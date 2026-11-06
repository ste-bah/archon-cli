//! Reverse check for declarative macros whose arguments live outside a closure.
use super::{Closure, Res, Workspace};
use std::collections::BTreeSet;
use std::path::PathBuf;

impl Workspace {
    /// Literal names emitted by a macro do not identify the invocation that
    /// supplies its arguments. Refuse a partial closure rather than certify a
    /// digest that omits those arguments. Scan every workspace module, including
    /// crates the forward walk never needed. Aliases are tracked conservatively
    /// across imports/reexports; an ambiguous same-name call is also required.
    pub(super) fn check_macro_invocations(
        &mut self,
        items: &BTreeSet<(PathBuf, usize)>,
        closure: &Closure,
    ) {
        let mut names = items
            .iter()
            .filter_map(|(file, at)| self.facts[file].items[*at].macro_definition.clone())
            .collect::<BTreeSet<_>>();
        if names.is_empty() {
            return;
        }
        for krate in self.roots.keys().cloned().collect::<Vec<_>>() {
            self.load(&krate);
        }
        loop {
            let before = names.len();
            for facts in self.facts.values() {
                for import in &facts.uses {
                    if import.path.last().is_some_and(|name| names.contains(name)) {
                        if let Some(alias) = &import.alias {
                            names.insert(alias.clone());
                        }
                    }
                }
            }
            if names.len() == before {
                break;
            }
        }
        let mut candidates = Vec::new();
        for (krate, tree) in &self.crates {
            for (file, module) in &tree.files {
                let facts = &self.facts[file];
                if facts.test_only || closure.files.contains(&self.rel(file)) {
                    continue;
                }
                for item in &facts.items {
                    for invocation in &item.macro_invocations {
                        if invocation.last().is_some_and(|name| names.contains(name)) {
                            candidates.push((
                                krate.clone(),
                                file.clone(),
                                [module.clone(), item.scope.clone()].concat(),
                                invocation.clone(),
                            ));
                        }
                    }
                }
            }
        }
        for (krate, file, scope, invocation) in candidates {
            // A different local macro with the same name is not an invocation
            // of the reached definition. Require resolved macro identity to
            // prove that exclusion; unresolved names remain an unsound closure.
            if let Ok(Res::Items(targets)) = self.resolve_path(&krate, &scope, &invocation) {
                let definitions = targets
                    .iter()
                    .filter(|(_, file, at)| self.facts[file].items[*at].macro_definition.is_some())
                    .collect::<Vec<_>>();
                if !definitions.is_empty()
                    && definitions
                        .iter()
                        .all(|(_, file, at)| !items.contains(&(file.clone(), *at)))
                {
                    continue;
                }
            }
            self.errors.insert(format!(
                "{}: macro invocation {}! is outside the closure of its reached definition; include the invocation module among the roots",
                self.rel(&file).display(), invocation.join("::")
            ));
        }
    }
}
