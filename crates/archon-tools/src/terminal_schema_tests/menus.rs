//! Which shells a world puts on the menu, and which shell a bare request gets.
//!
//! `every_advertised_shell_opens_and_every_omitted_one_is_refused` carries the
//! file: it runs each menu back through `terminal_world::plan` — the call
//! `execute` makes — and checks not just that an advertised shell opens but
//! that it opens in the world the menu said it would.

use super::*;

#[test]
fn the_host_schema_offers_every_shell_and_names_the_platform_default() {
    let built = host_schema();

    assert_eq!(advertised(&built), shells::SHELLS.to_vec());
    assert!(
        described(&built).contains(shells::default_shell()),
        "{}",
        described(&built)
    );
}

/// The pin on the no-sandbox case. A context with no world of its own declares
/// nothing, so the surface an unsandboxed session sees is not rebuilt — it is
/// untouched.
#[test]
fn a_session_with_no_backend_is_described_exactly_as_before() {
    assert_eq!(world_schema(&ctx(None)), None);
    assert_eq!(
        world_schema(&ctx(Some(FixedTerminalBackend::host()))),
        None,
        "a policy-only backend runs host shells, so it has nothing to re-describe"
    );
}

#[test]
fn a_linux_world_advertises_only_the_shells_it_has() {
    let built = world_schema(&linux()).expect("a container is not a host");

    assert_eq!(advertised(&built), vec!["bash", "sh"]);
    assert!(
        !described(&built).contains("powershell") && !described(&built).contains("cmd"),
        "{}",
        described(&built)
    );
}

/// The default differs per world: the host default on Windows is PowerShell,
/// and promising that to a Linux container would refuse every terminal the
/// model opened without naming a shell.
///
/// Asked against a world whose answer is `sh` — neither platform's host default
/// — so the assertion bites wherever it runs rather than only on Windows.
#[test]
fn the_advertised_default_is_the_one_a_bare_request_actually_gets() {
    let ctx = ctx(Some(Arc::new(PosixShWorld)));
    let built = world_schema(&ctx).expect("a container is not a host");

    let opened = plan(&ctx, None, &ctx.working_dir).expect("a bare request opens");
    assert_eq!(opened.shell, "sh");
    assert_ne!(
        opened.shell,
        shells::default_shell(),
        "the point of this world is that it disagrees with the host"
    );
    assert!(
        described(&built).contains(&format!("default {}", opened.shell)),
        "schema says {:?}, the call opens {}",
        described(&built),
        opened.shell
    );
}

/// The same for the menu: a world that has `bash` and `sh` but defaults to `sh`
/// pins the default independently of which shells are on offer.
#[test]
fn a_worlds_default_is_pinned_apart_from_its_menu() {
    let ctx = ctx(Some(Arc::new(PosixShWorld)));
    let built = world_schema(&ctx).expect("a container is not a host");

    assert_eq!(advertised(&built), vec!["bash", "sh"]);
    assert!(
        described(&built).contains("default sh"),
        "a menu containing bash must not make bash the default: {}",
        described(&built)
    );
}

/// The whole point, stated as a round trip. Everything the schema offers must
/// open, and everything it leaves out must be refused — checked against `plan`,
/// which is the function `TerminalCreateTool::execute` calls.
#[test]
fn every_advertised_shell_opens_and_every_omitted_one_is_refused() {
    for backend in [
        Arc::new(LinuxWorld) as Arc<dyn SandboxBackend>,
        Arc::new(PosixShWorld),
        Arc::new(SandboxWithOneHostShell),
        Arc::new(HostWithOneSandboxedShell),
        Arc::new(HostMinusWindowsShells),
    ] {
        let ctx = ctx(Some(backend));
        let built = world_schema(&ctx).expect("none of these worlds is a plain host");
        let offered = advertised(&built);
        let sandboxed_menu = described(&built).contains("in a sandbox");

        for shell in shells::SHELLS {
            let is_offered = offered.iter().any(|name| name == shell);
            let Ok(launch) = plan(&ctx, Some(shell), &ctx.working_dir) else {
                assert!(!is_offered, "{shell} is advertised but refused");
                continue;
            };
            // Opening is not enough. A menu that says these shells run in a
            // sandbox must not contain one that opens on the host, which is
            // the failure a bare `is_ok()` check would have waved through.
            assert_eq!(
                is_offered,
                launch.sandboxed == sandboxed_menu,
                "{shell}: advertised={is_offered}, but it opens {}",
                if launch.sandboxed {
                    "inside the sandbox"
                } else {
                    "on the host"
                }
            );
        }
    }
}

/// The mirror of the case above, and the one that matters more. A world that
/// relocates its shells may still answer `Host` for one of them; advertising
/// that shell under a sandboxed menu would tell the model it is isolated when
/// `plan` opens it on the machine.
#[test]
fn a_shell_that_escapes_to_the_host_is_not_advertised_as_sandboxed() {
    let ctx = ctx(Some(Arc::new(SandboxWithOneHostShell)));
    let built = world_schema(&ctx).expect("a relocating world is not a host");

    assert!(
        described(&built).contains("in a sandbox"),
        "{}",
        described(&built)
    );
    assert!(
        !advertised(&built).contains(&"sh".to_string()),
        "sh opens on the host here and must not sit on a sandboxed menu: {:?}",
        advertised(&built)
    );
    let escaped = plan(&ctx, Some("sh"), &ctx.working_dir).expect("the host still opens it");
    assert!(
        !escaped.sandboxed,
        "the fake must actually escape, or this test proves nothing"
    );
}

/// And the same defect pointing the other way: a host menu must not carry a
/// shell that lands inside a sandbox, because the prose says the opposite.
#[test]
fn a_host_menu_does_not_carry_a_shell_that_lands_in_a_sandbox() {
    let ctx = ctx(Some(Arc::new(HostWithOneSandboxedShell)));
    let built = world_schema(&ctx).expect("one shell was moved, so there is something to say");

    assert!(
        !described(&built).contains("in a sandbox"),
        "these are host shells: {}",
        described(&built)
    );
    assert_eq!(advertised(&built), vec!["sh"]);
    let relocated = plan(&ctx, Some("bash"), &ctx.working_dir).expect("the sandbox opens it");
    assert!(
        relocated.sandboxed,
        "the fake must actually relocate, or this test proves nothing"
    );
}

/// A host shell for the bare request does not license the whole host menu. The
/// backend is asked about each shell separately, because it is allowed to
/// answer differently — and here it does.
#[test]
fn a_world_that_hosts_shells_but_bans_two_advertises_only_the_rest() {
    let ctx = ctx(Some(Arc::new(HostMinusWindowsShells)));
    let built =
        world_schema(&ctx).expect("two shells were taken away, so there is something to say");

    assert_eq!(advertised(&built), vec!["bash", "sh"]);
    for shell in ["powershell", "cmd"] {
        assert!(
            plan(&ctx, Some(shell), &ctx.working_dir).is_err(),
            "{shell} is advertised nowhere and must open nowhere"
        );
    }
}

/// A world can only narrow the menu, never add to it: `offer` filters
/// `shells::SHELLS`, the same list the host launcher builds from, so a backend
/// cannot advertise a shell nothing here knows how to name.
#[test]
fn no_world_can_advertise_a_shell_the_launcher_does_not_know() {
    for backend in [
        Arc::new(LinuxWorld) as Arc<dyn SandboxBackend>,
        FixedTerminalBackend::opening(door()),
    ] {
        let Some(built) = world_schema(&ctx(Some(backend))) else {
            continue;
        };
        for shell in advertised(&built) {
            assert!(
                shells::SHELLS.contains(&shell.as_str()),
                "{shell} is not a shell this build can launch"
            );
        }
    }
}
