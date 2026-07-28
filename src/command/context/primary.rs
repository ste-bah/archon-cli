use crate::command::registry::Registry;

/// Resolve a slash `input` to its primary command name, consulting the
/// [`Registry`]'s alias map. Returns `None` for non-slash inputs, parse
/// failures, or unknown command names.
///
/// Private to this module: builder tests exercise it directly against
/// `default_registry()` because stubbing the full `SlashCommandContext`
/// is out of scope for this ticket (see AGS-807 executor report).
pub(crate) fn resolve_primary_from_input(input: &str, registry: &Registry) -> Option<String> {
    // Reuse the shared tokenizer so we inherit its leading-`/` handling,
    // quoted-arg rules, and flag tolerance.
    let parsed = crate::command::parser::CommandParser::parse(input).ok()?;

    // `Registry::get` is alias-aware; we re-derive the PRIMARY name by
    // asking the commands map first, then the alias map, so the caller
    // can compare against a canonical string like "status".
    if registry.get(&parsed.name).is_some() {
        // Direct primary hit: return the parsed name itself. But if the
        // name is an alias (not a primary), we need the primary string.
        if registry.is_primary(&parsed.name) {
            return Some(parsed.name);
        }
        // Alias → primary resolution.
        return registry.primary_for_alias(&parsed.name).map(str::to_string);
    }
    None
}
