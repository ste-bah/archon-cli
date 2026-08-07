# Hooks

**This page has moved to [`docs/reference/hooks.md`](../reference/hooks.md).**

The content that used to live here described an interface that does not exist:
a `[[hooks.pre_tool_use]]` TOML shape, a `blocking = true` field, object
matchers with `tool_name` and `path_regex`, and a `/hooks add` subcommand. None
of those are in the source. It was wrong rather than merely out of date, which
is worse — a reader following it would write a config that silently never runs.

The replacement is generated from the source and records what the runtime
actually does with each event's result, which is the part you need in order to
choose an event.
