//! What a world says when it will not open a terminal, and the argument shape
//! every world shares.
//!
//! A refusal has to reach the tool description: `TerminalCreate` requires no
//! arguments, so the call a model makes reads no argument description on the
//! way there.

use super::*;

/// `TerminalCreate` requires no arguments, so `TerminalCreate {}` is the call a
/// model makes — and it reads no argument description on the way. A refusal
/// that lived only in `properties.shell.description` was invisible to exactly
/// the call it needed to stop.
#[test]
fn a_refusal_reaches_a_call_that_names_no_arguments() {
    let ctx = ctx(Some(FixedTerminalBackend::refusing(
        "openshell sandbox: throwaway sandbox per command",
    )));
    let built = world_schema(&ctx).expect("a refusal is something to say");

    // The shape a zero-argument call is checked against is still satisfiable,
    // so nothing about the schema stops it.
    assert_eq!(built["required"], serde_json::json!([]));

    let description = world_description(&ctx, "WHAT IT NORMALLY DOES")
        .expect("a world with no terminal changes what this tool is");
    assert!(description.contains("UNAVAILABLE"), "{description}");
    assert!(
        description.contains("throwaway sandbox per command"),
        "{description}"
    );
    assert!(
        description.contains("no arguments"),
        "the description has to say the empty call is refused too: {description}"
    );
    assert!(
        description.contains("WHAT IT NORMALLY DOES"),
        "a refusal must not erase what the tool is: {description}"
    );
    assert!(plan(&ctx, None, &ctx.working_dir).is_err());
}

/// Only a refusal changes what the tool *is*. A narrowed shell menu is an
/// argument detail, and rewriting the description for it would churn the
/// prompt-cache prefix for something the schema already says.
#[test]
fn a_narrowed_menu_leaves_the_description_alone() {
    assert_eq!(world_description(&ctx(None), "base"), None);
    assert_eq!(world_description(&linux(), "base"), None);
    assert_eq!(
        world_description(&ctx(Some(FixedTerminalBackend::host())), "base"),
        None
    );
}

/// A backend decides on the directory as well as the shell, and `offer` is only
/// ever asked about the session working directory. A shell being on the menu
/// therefore says nothing about a `cwd` argument, and the argument has to say
/// so itself rather than let the menu imply an all-clear.
#[test]
fn a_sandboxed_cwd_says_the_world_may_still_refuse_it() {
    let sandboxed = world_schema(&linux()).expect("a container is not a host");
    let host_text = host_schema()["properties"]["cwd"]["description"].clone();

    let described = sandboxed["properties"]["cwd"]["description"]
        .as_str()
        .expect("cwd is described");
    assert!(described.contains("refuse a directory"), "{described}");
    assert_ne!(sandboxed["properties"]["cwd"]["description"], host_text);

    // A world that narrows the menu without relocating anything keeps the host
    // text: there is no second filesystem for it to warn about.
    let banned = world_schema(&ctx(Some(Arc::new(HostMinusWindowsShells))))
        .expect("two shells were taken away");
    assert_eq!(banned["properties"]["cwd"]["description"], host_text);
}

#[test]
fn a_world_that_cannot_host_a_shell_says_so_instead_of_offering_four() {
    let built = world_schema(&ctx(Some(FixedTerminalBackend::refusing(
        "openshell sandbox: throwaway sandbox per command",
    ))))
    .expect("a refusal is something to say");

    assert!(advertised(&built).is_empty());
    assert!(
        described(&built).contains("throwaway sandbox per command"),
        "{}",
        described(&built)
    );
}

#[test]
fn a_world_that_accepts_no_named_shell_offers_no_enum_at_all() {
    let ctx = ctx(Some(Arc::new(NamelessWorld)));
    let built = world_schema(&ctx).expect("it is not a host");

    assert!(
        shell_property(&built).get("enum").is_none(),
        "an empty enum matches nothing and providers reject it"
    );
    assert!(described(&built).contains("Omit"), "{}", described(&built));
    assert!(plan(&ctx, Some("bash"), &ctx.working_dir).is_err());
    assert!(plan(&ctx, None, &ctx.working_dir).is_ok());
}

/// Every world describes the same argument *shape* — same names, same types,
/// same nothing-required. Only the prose differs, so a model that learned how
/// to call this tool in one session does not have to relearn it in the next.
#[test]
fn the_argument_shape_is_the_same_in_every_world() {
    let host = host_schema();
    let worlds = [
        world_schema(&linux()).expect("a container is not a host"),
        world_schema(&ctx(Some(Arc::new(PosixShWorld)))).expect("nor is this one"),
        world_schema(&ctx(Some(FixedTerminalBackend::refusing("no session"))))
            .expect("a refusal still describes the arguments"),
    ];

    for built in std::iter::once(&host).chain(worlds.iter()) {
        assert_eq!(built["type"], "object");
        assert_eq!(built["required"], serde_json::json!([]));
        assert_eq!(shell_property(built)["type"], "string");
        assert_eq!(built["properties"]["cwd"]["type"], "string");
        assert_eq!(
            built["properties"].as_object().expect("an object").len(),
            2,
            "no world may add or drop an argument"
        );
    }
}

#[test]
fn the_tool_itself_answers_with_the_world_it_is_asked_about() {
    use crate::terminal_tools::TerminalCreateTool;
    use crate::tool::Tool;

    assert_eq!(TerminalCreateTool.input_schema(), host_schema());
    assert_eq!(TerminalCreateTool.input_schema_for(&ctx(None)), None);
    assert_eq!(
        TerminalCreateTool.input_schema_for(&linux()),
        world_schema(&linux())
    );
    assert_eq!(TerminalCreateTool.description_for(&ctx(None)), None);
    let refusing = ctx(Some(FixedTerminalBackend::refusing("no session")));
    assert_eq!(
        TerminalCreateTool.description_for(&refusing),
        world_description(&refusing, TerminalCreateTool.description())
    );
}
