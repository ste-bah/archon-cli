use super::context::CommandContext;

/// Trait every registered slash command handler implements.
///
/// `execute` runs the handler against the supplied context and
/// positional argument list. `description` is a one-line human label
/// used by `/help`, the command picker, and future introspection.
///
/// TASK-AGS-802: `aliases()` returns zero or more alternative names
/// the registry routes to the same handler. Default `&[]` keeps every
/// pre-existing handler wire-compatible — only handlers that opt in by
/// overriding the method contribute to the alias map.
pub(crate) trait CommandHandler: Send + Sync {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()>;
    fn description(&self) -> &str;

    /// Alternative names that resolve to this handler. The registry
    /// builds an alias -> primary-name map at init time; `Registry::get`
    /// falls back to that map when the direct lookup misses.
    ///
    /// Default empty slice: handlers that do not declare aliases do not
    /// contribute any entries. No allocations at call time.
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}
