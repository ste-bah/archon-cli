use super::*;

/// Minimal handler used by collision tests (test-local, no real body).
struct TestHandler {
    desc: &'static str,
    aliases: &'static [&'static str],
}

impl CommandHandler for TestHandler {
    fn execute(&self, _ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        Ok(())
    }

    fn description(&self) -> &str {
        self.desc
    }

    fn aliases(&self) -> &'static [&'static str] {
        self.aliases
    }
}

/// Handler with no alias override — exercises the default empty-slice
/// implementation on the trait.
struct NoAliasHandler;

impl CommandHandler for NoAliasHandler {
    fn execute(&self, _ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        Ok(())
    }

    fn description(&self) -> &str {
        "no-alias handler (test only)"
    }
}

#[test]
fn default_registry_resolves_alias_to_primary() {
    // "h" is an alias for "help" in the starter set. Resolution
    // must return the SAME handler (same description) as the
    // primary lookup.
    let registry = default_registry();
    let via_primary = registry.get("help").expect("primary help registered");
    let via_alias = registry.get("h").expect("alias h resolves to help");
    assert_eq!(
        via_primary.description(),
        via_alias.description(),
        "alias must resolve to same handler as primary"
    );
}

#[test]
fn default_registry_alias_count_minimum() {
    let registry = default_registry();
    assert!(
        registry.alias_count() >= 8,
        "starter set must have >= 8 aliases, got {}",
        registry.alias_count()
    );
}

#[test]
#[should_panic(expected = "duplicate primary")]
fn duplicate_primary_name_panics() {
    // Two primaries with the same name must panic at build time.
    let mut b = RegistryBuilder::new();
    b.insert_primary("dup", Arc::new(NoAliasHandler));
    b.insert_primary("dup", Arc::new(NoAliasHandler));
    let _ = b.build();
}

#[test]
#[should_panic(expected = "alias collides with primary")]
fn alias_collides_with_primary_panics() {
    // An alias equal to an existing primary name must panic.
    let h = Arc::new(TestHandler {
        desc: "has alias 'existing'",
        aliases: &["existing"],
    });
    let other = Arc::new(NoAliasHandler);
    let mut b = RegistryBuilder::new();
    b.insert_primary("existing", other);
    b.insert_primary("mycmd", h);
    let _ = b.build();
}

#[test]
#[should_panic(expected = "duplicate alias")]
fn alias_collides_with_alias_panics() {
    // Two handlers claiming the same alias must panic.
    let a = Arc::new(TestHandler {
        desc: "handler a",
        aliases: &["shared"],
    });
    let b_h = Arc::new(TestHandler {
        desc: "handler b",
        aliases: &["shared"],
    });
    let mut b = RegistryBuilder::new();
    b.insert_primary("alpha", a);
    b.insert_primary("beta", b_h);
    let _ = b.build();
}

#[test]
fn registry_len_counts_primaries_only() {
    // Aliases must NOT inflate the primary count.
    let registry = default_registry();
    assert_eq!(
        registry.len(),
        EXPECTED_COMMAND_COUNT,
        "len() must count primaries only, not primaries + aliases"
    );
}

#[test]
fn registry_names_returns_all_primaries() {
    let registry = default_registry();
    let names = registry.names();
    assert_eq!(
        names.len(),
        EXPECTED_COMMAND_COUNT,
        "names() must return one entry per primary command"
    );
    // Spot-check a few well-known primaries.
    assert!(names.contains(&"help"));
    assert!(names.contains(&"recall"));
    assert!(names.contains(&"config"));
}

#[test]
fn recall_is_standalone_not_alias() {
    // /recall stays a primary command and is NOT registered as an
    // alias for anything (Steven directive).
    let registry = default_registry();
    let handler = registry.get("recall").expect("recall is a primary");
    assert!(
        handler.description().to_lowercase().contains("recall")
            || handler.description().to_lowercase().contains("memor"),
        "recall handler description should reference recall/memory, got: {}",
        handler.description()
    );
    assert!(
        !registry.aliases_map_contains("recall"),
        "recall must NOT appear as an alias"
    );
}

// -----------------------------------------------------------------
// TASK-AGS-805: /cancel registration + aliases (stop, abort).
// Body-migrate is deferred until CommandContext exposes a task
// service; these tests verify the registry-level wiring only.
// -----------------------------------------------------------------

#[test]
fn cancel_primary_registered() {
    let registry = default_registry();
    let handler = registry
        .get("cancel")
        .expect("cancel must be registered as a primary");
    assert!(
        !handler.description().is_empty(),
        "cancel handler must carry a non-empty description"
    );
}

// -----------------------------------------------------------------
// TASK-AGS-807: /status alias `info` resolves to the /status handler.
// -----------------------------------------------------------------

#[test]
fn registry_resolves_status_alias_info() {
    let reg = default_registry();
    let primary = reg
        .get("status")
        .expect("status primary must be registered");
    let via_info = reg
        .get("info")
        .expect("'info' alias must resolve to /status per AGS-807");
    assert_eq!(
        primary.description(),
        via_info.description(),
        "'info' must resolve to the same handler as /status"
    );
    // Also pin the Registry helper APIs introduced for the
    // builder's alias-aware primary-name resolution.
    assert!(reg.is_primary("status"));
    assert!(!reg.is_primary("info"));
    assert_eq!(reg.primary_for_alias("info"), Some("status"));
    assert_eq!(reg.primary_for_alias("status"), None);
}

#[test]
fn cancel_aliases_resolve_to_cancel_handler() {
    let registry = default_registry();
    let primary = registry.get("cancel").expect("cancel primary registered");
    let via_stop = registry
        .get("stop")
        .expect("alias 'stop' must resolve to cancel");
    let via_abort = registry
        .get("abort")
        .expect("alias 'abort' must resolve to cancel");
    assert_eq!(
        primary.description(),
        via_stop.description(),
        "'stop' must resolve to the same handler as /cancel"
    );
    assert_eq!(
        primary.description(),
        via_abort.description(),
        "'abort' must resolve to the same handler as /cancel"
    );
}

// -----------------------------------------------------------------
// TASK-AGS-808: /model aliases [m, switch-model] + CommandEffect
// enum sanity. The /model body-migrate moves ModelHandler out of
// the declare_handler! stub and into `crate::command::model`.
// -----------------------------------------------------------------

#[test]
fn registry_resolves_model_aliases_m_and_switch_model() {
    let reg = default_registry();
    let primary = reg.get("model").expect("model primary must be registered");
    let via_m = reg
        .get("m")
        .expect("'m' alias must resolve to /model per AGS-808");
    let via_switch_model = reg
        .get("switch-model")
        .expect("'switch-model' alias must resolve to /model per AGS-808");
    assert_eq!(
        primary.description(),
        via_m.description(),
        "'m' must resolve to the same handler as /model"
    );
    assert_eq!(
        primary.description(),
        via_switch_model.description(),
        "'switch-model' must resolve to the same handler as /model"
    );
    // Pin the Registry helper APIs — `model` is a primary,
    // `m` is not.
    assert!(reg.is_primary("model"));
    assert!(!reg.is_primary("m"));
    assert!(!reg.is_primary("switch-model"));
    assert_eq!(reg.primary_for_alias("m"), Some("model"));
    assert_eq!(reg.primary_for_alias("switch-model"), Some("model"));
    assert_eq!(reg.primary_for_alias("model"), None);
}

// -----------------------------------------------------------------
// TASK-AGS-809: /cost aliases [billing] (collision-adjusted from
// the spec-requested [usage, billing] — see cost.rs rustdoc for
// the CONFIRM R-item: `usage` is already a shipped primary).
// -----------------------------------------------------------------

#[test]
fn registry_resolves_cost_aliases_usage_and_billing() {
    let reg = default_registry();
    let primary = reg.get("cost").expect("cost primary must be registered");
    let via_billing = reg
        .get("billing")
        .expect("'billing' alias must resolve to /cost per AGS-809");
    assert_eq!(
        primary.description(),
        via_billing.description(),
        "'billing' must resolve to the same handler as /cost"
    );

    // `usage` stays a PRIMARY (UsageHandler) — must NOT resolve to
    // /cost. Enforces the collision-avoidance invariant.
    let via_usage = reg
        .get("usage")
        .expect("'usage' must still resolve — it is a shipped primary");
    assert_ne!(
        primary.description(),
        via_usage.description(),
        "'usage' must remain bound to UsageHandler, not /cost"
    );

    // Pin the Registry helper APIs — `cost` and `usage` are BOTH
    // primaries (independent); `billing` is the only /cost alias.
    assert!(reg.is_primary("cost"));
    assert!(reg.is_primary("usage"));
    assert!(!reg.is_primary("billing"));
    assert_eq!(reg.primary_for_alias("billing"), Some("cost"));
    assert_eq!(reg.primary_for_alias("usage"), None);
    assert_eq!(reg.primary_for_alias("cost"), None);
}

// -----------------------------------------------------------------
// TASK-AGS-810: /resume aliases [continue, open-session] resolve.
// DIRECT-pattern body-migrate — no snapshot or effect slot. This
// test pins the alias surface so future ticketing cannot silently
// drop `open-session` (AGS-810 spec validation criterion 4).
// -----------------------------------------------------------------

#[test]
fn registry_resolves_resume_aliases_continue_and_open_session() {
    let reg = default_registry();
    let primary = reg
        .get("resume")
        .expect("resume primary must be registered");
    let via_continue = reg
        .get("continue")
        .expect("'continue' alias must resolve to /resume");
    let via_open_session = reg
        .get("open-session")
        .expect("'open-session' alias must resolve to /resume per AGS-810");
    assert_eq!(
        primary.description(),
        via_continue.description(),
        "'continue' must resolve to the same handler as /resume"
    );
    assert_eq!(
        primary.description(),
        via_open_session.description(),
        "'open-session' must resolve to the same handler as /resume"
    );

    // Pin the Registry helper APIs — `resume` is a primary,
    // `continue` and `open-session` are aliases (not primaries).
    assert!(reg.is_primary("resume"));
    assert!(!reg.is_primary("continue"));
    assert!(!reg.is_primary("open-session"));
    assert_eq!(reg.primary_for_alias("continue"), Some("resume"));
    assert_eq!(reg.primary_for_alias("open-session"), Some("resume"));
    assert_eq!(reg.primary_for_alias("resume"), None);
}
