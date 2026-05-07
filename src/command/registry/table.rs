use std::collections::HashMap;
use std::sync::Arc;

use super::handler::CommandHandler;

/// Typed command table.
///
/// Owns `Arc<dyn CommandHandler>` so the dispatcher can clone handlers
/// out of the map cheaply and invoke them without holding a borrow on
/// the registry. Insertion order is irrelevant; lookup is by name.
///
/// TASK-AGS-802: an `aliases` map routes alternative names onto their
/// primary command. `get()` consults `commands` first, then falls back
/// to `aliases` for alias -> primary -> handler resolution. The alias
/// map does NOT inflate `len()`; `alias_count()` reports the alias
/// total separately.
pub(crate) struct Registry {
    commands: HashMap<&'static str, Arc<dyn CommandHandler>>,
    aliases: HashMap<&'static str, &'static str>,
}

impl Registry {
    /// Look up a registered handler by command name (without the
    /// leading `/`). Returns a cloned `Arc`, or `None` if no handler
    /// is registered under that name.
    ///
    /// Resolution order: primary-name map first, then alias map.
    /// Aliases resolve by looking up the primary name they target and
    /// re-reading the commands map.
    pub(crate) fn get(&self, name: &str) -> Option<Arc<dyn CommandHandler>> {
        if let Some(h) = self.commands.get(name) {
            return Some(Arc::clone(h));
        }
        let primary = self.aliases.get(name)?;
        self.commands.get(primary).cloned()
    }

    /// Number of registered primary commands. Aliases are counted
    /// separately (see [`Registry::alias_count`]).
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.commands.len()
    }

    /// Number of registered aliases (not counted against `len()`).
    #[allow(dead_code)]
    pub(crate) fn alias_count(&self) -> usize {
        self.aliases.len()
    }

    /// All primary command names, in unspecified order. Used by the
    /// dispatcher's unknown-command path to feed
    /// [`crate::command::parser::suggest`] with the list of candidates,
    /// and reused by TASK-AGS-804 for fuzzy-match hints.
    pub(crate) fn names(&self) -> Vec<&'static str> {
        self.commands.keys().copied().collect()
    }

    /// All primary command names paired with their registered descriptions.
    /// Mirrors [`Registry::names`] shape — iterates `self.commands` and calls
    /// [`CommandHandler::description`] for each entry. Used to build the TUI
    /// autocomplete catalog so the popup stays locked to the registry.
    pub(crate) fn primaries_with_descriptions(&self) -> Vec<(&'static str, &str)> {
        self.commands
            .iter()
            .map(|(name, handler)| (*name, handler.description()))
            .collect()
    }

    /// Test-only helper: returns `true` if `alias` is registered in the
    /// alias map. The `recall_is_standalone_not_alias` test uses this
    /// to enforce Steven's directive that `/recall` stays a primary
    /// command and is never an alias for anything.
    #[cfg(test)]
    pub(crate) fn aliases_map_contains(&self, alias: &str) -> bool {
        self.aliases.contains_key(alias)
    }

    /// TASK-AGS-807 helper: returns `true` if `name` is registered as a
    /// PRIMARY command (not just reachable via the alias map).
    ///
    /// Used by `crate::command::context::resolve_primary_from_input`
    /// to decide whether the parsed input name is already the primary
    /// or needs an alias→primary lookup.
    pub(crate) fn is_primary(&self, name: &str) -> bool {
        self.commands.contains_key(name)
    }

    /// TASK-AGS-807 helper: map an alias to its primary command name.
    /// Returns `None` if `alias` is not registered in the alias map.
    ///
    /// Alias entries are internalized as `&'static str`, so we can
    /// return a borrowed static reference without cloning.
    pub(crate) fn primary_for_alias(&self, alias: &str) -> Option<&'static str> {
        self.aliases.get(alias).copied()
    }
}

// ---------------------------------------------------------------------------
// Registry builder (init-time assembly + collision detection)
// ---------------------------------------------------------------------------

/// Assembles a [`Registry`] with alias support and panics on any of
/// three collision classes at build time:
///
/// 1. **Primary/primary**: two primaries sharing the same name.
/// 2. **Alias/primary**: an alias whose string equals an existing
///    primary name.
/// 3. **Alias/alias**: two handlers claiming the same alias.
///
/// Insertion order matters: callers must insert ALL primaries before
/// any aliases so the alias-vs-primary check can see every primary
/// name in the commands map. `build()` enforces this by walking every
/// primary handler's `aliases()` method after primaries are frozen.
pub(crate) struct RegistryBuilder {
    commands: HashMap<&'static str, Arc<dyn CommandHandler>>,
    primary_order: Vec<&'static str>,
}

impl RegistryBuilder {
    pub(crate) fn new() -> Self {
        Self {
            commands: HashMap::new(),
            primary_order: Vec::new(),
        }
    }

    /// Insert a primary command. Panics if the name is already
    /// registered.
    pub(crate) fn insert_primary(&mut self, name: &'static str, handler: Arc<dyn CommandHandler>) {
        if self.commands.contains_key(name) {
            panic!("duplicate primary slash command: /{name} registered twice");
        }
        self.commands.insert(name, handler);
        self.primary_order.push(name);
    }

    /// Freeze the commands map, walk every handler's `aliases()`,
    /// build the alias index, and detect alias/primary and alias/alias
    /// collisions. Panics on any collision.
    pub(crate) fn build(self) -> Registry {
        let Self {
            commands,
            primary_order,
        } = self;
        let mut aliases: HashMap<&'static str, &'static str> = HashMap::new();
        for primary in &primary_order {
            let handler = commands
                .get(primary)
                .expect("primary registered via insert_primary");
            for alias in handler.aliases() {
                if commands.contains_key(alias) {
                    panic!(
                        "alias collides with primary: alias '{alias}' (on /{primary}) matches existing primary command /{alias}"
                    );
                }
                if let Some(prior) = aliases.get(alias) {
                    panic!("duplicate alias: '{alias}' registered by both /{prior} and /{primary}");
                }
                aliases.insert(alias, primary);
            }
        }
        Registry { commands, aliases }
    }
}
