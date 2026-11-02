//! A failed host listing is evidence of a site lack only with a host plugin.

use std::path::Path;

use super::super::super::{ExecutabilityProbe, HostProbe, UnprovenCause, silent};
use super::super::no_verdict_cause;
use super::tests::Bin;
use super::tests_333b::contract;

fn no_answer(host_has_plugin: bool) {
    let installed = [("fixture-item", "#!/bin/sh\nexit 0\n")];
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[(
            "fixture",
            "#!/bin/sh\nif [ \"$1\" = --list ]; then echo 'listing probe failed' >&2; exit 126; fi\necho \"unknown command '$1'\" >&2\nexit 1\n",
        )],
        if host_has_plugin { &installed } else { &[] },
    );
    let mut run = bin.run("fixture item");
    run.acceptance_id = "AC-X-001".into();
    assert_eq!(run.exit_code, Some(1));
    assert!(String::from_utf8_lossy(&run.stderr).contains("unknown command 'item'"));
    let at = bin.context();
    let (why, cause) = no_verdict_cause("fixture item", &run, &at).unwrap();
    assert!(matches!(
        super::list::last_listing(&bin.dir.path().join("fixture")),
        Some(super::Listing::NoAnswer(_))
    ));
    let expected = if host_has_plugin {
        UnprovenCause::SiteLacks
    } else {
        UnprovenCause::TransientHost
    };
    assert_eq!(cause, expected, "{why}");
    if host_has_plugin {
        assert!(why.contains(bin.host.path().to_str().unwrap()), "{why}");
    } else {
        assert!(
            why.contains("gives no evidence that the check or its site is at fault"),
            "{why}"
        );
    }
    let mut check = contract("fixture item");
    check.acceptance[0].id = run.acceptance_id.clone();
    let root = tempfile::tempdir().unwrap();
    for _ in 0..2 {
        let probe = HostProbe::at(root.path().into(), root.path().into(), None);
        let finding = silent::settle_as(&probe, &"0".repeat(40), &check, &run, (&why, cause), &at);
        assert_eq!(finding.is_some(), host_has_plugin, "{finding:?}");
        let held = probe.take_unproven_causes();
        if host_has_plugin {
            assert!(held.is_empty(), "{held:?}");
            assert!(
                finding
                    .unwrap()
                    .contains("Check the criterion with what the site provides")
            );
        } else {
            assert_eq!(held["AC-X-001"].1, UnprovenCause::TransientHost);
        }
        assert_eq!(probe.resume.progress.total(), 0);
    }
}

#[test]
fn no_answer_without_a_host_plugin_is_transient_without_an_author_finding() {
    no_answer(false);
}

#[test]
fn no_answer_with_a_host_plugin_is_site_lacks_and_reaches_its_author() {
    no_answer(true);
}

fn scratch_overrun(host_has_plugin: bool) {
    let installed = [("fixture-item", "#!/bin/sh\nexit 0\n")];
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[(
            "fixture",
            "#!/bin/sh\nif [ \"$1\" = --list ]; then /usr/bin/head -c 17000000 /dev/zero > \"$TMPDIR/fill\"; exec /bin/sleep 30; fi\nif [ -f \"$2\" ]; then exit 0; fi\necho \"error: unknown command '$1'\" >&2\nexit 1\n",
        )],
        if host_has_plugin { &installed } else { &[] },
    );
    let text = "fixture item data/state.txt";
    let mut run = bin.run(text);
    run.acceptance_id = "AC-X-001".into();
    assert_eq!(run.exit_code, Some(1));
    assert!(String::from_utf8_lossy(&run.stderr).contains("error: unknown command 'item'"));
    let at = bin.context();
    let cause = no_verdict_cause(text, &run, &at);
    assert!(matches!(
        super::list::last_listing(&bin.dir.path().join("fixture")),
        Some(super::Listing::ScratchUnproven(_))
    ));
    if host_has_plugin {
        let (why, cause) = cause.as_ref().expect("the host provides a plugin");
        assert_eq!(*cause, UnprovenCause::SiteLacks, "{why}");
        assert!(
            why.contains(bin.host.path().join("fixture-item").to_str().unwrap()),
            "{why}"
        );
    } else {
        assert_eq!(cause, None, "a scratch overrun says nothing about the site");
    }
    let mut check = contract(text);
    check.acceptance[0].id = run.acceptance_id.clone();
    let root = tempfile::tempdir().unwrap();
    let probe = HostProbe::at(root.path().into(), root.path().into(), None);
    for _ in 0..2 {
        let finding = silent::silent_failure(&check, &run, &at).and_then(|why| {
            let (_, cause) = no_verdict_cause(text, &run, &at).unwrap();
            silent::settle_as(&probe, &"0".repeat(40), &check, &run, (&why, cause), &at)
        });
        assert_eq!(finding.is_some(), host_has_plugin, "{finding:?}");
        assert!(probe.take_unproven_causes().is_empty());
        assert_eq!(probe.resume.progress.total(), 0);
    }
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir(data.path().join("data")).unwrap();
    std::fs::write(data.path().join("data/state.txt"), "ready").unwrap();
    assert_eq!(
        bin.run(&format!("cd '{}' && {text}", data.path().display()))
            .exit_code,
        Some(0)
    );
}

#[test]
fn scratch_overrun_without_a_host_plugin_has_no_site_lacks_claim_or_author_finding() {
    scratch_overrun(false);
}

#[test]
fn scratch_overrun_with_a_host_plugin_is_site_lacks_and_reaches_its_author() {
    scratch_overrun(true);
}
