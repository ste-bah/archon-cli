use super::{host, no_verdict, run};

#[test]
fn shell_option_refusal_wordings_are_classified_without_running_a_shell() {
    // Sources: bash shell.c https://github.com/gitGNU/gnu_bash/blob/master/shell.c
    // dash options.c https://sources.debian.org/src/dash/0.5.10.2-5/src/options.c/
    // BusyBox ash.c https://coral.googlesource.com/busybox/+/refs/heads/master/shell/ash.c
    // ksh93 reproducer https://github.com/ksh93/ksh/issues/347
    for (command, stderr, expected) in [
        (
            "bash -z -c :",
            "bash: -z: invalid option\n",
            "before reading",
        ),
        ("sh -z -c :", "sh: 0: Illegal option -\n", "before reading"),
        ("ash -z -c :", "ash: illegal option -z\n", "before reading"),
        ("ksh -z -c :", "ksh: -z: unknown option\n", "before reading"),
        // Exact dash diagnostics requested for invocation and `set` refusal.
        (
            "dash -z -c :",
            "sh: 0: Illegal option -\n",
            "before reading",
        ),
        (
            "set -z\ntest -f deliverable",
            "sh: 1: set: Illegal option -\n",
            "before any criterion",
        ),
        (
            "set -z\ntest -f deliverable",
            "sh: set: illegal option -z\n",
            "before any criterion",
        ),
        (
            "set -z\ntest -f deliverable",
            "bash: set: -z: invalid option\n",
            "before any criterion",
        ),
        (
            "set -o posix\ntest -f deliverable",
            "ksh: set: posix: bad option(s)\n",
            "before any criterion",
        ),
    ] {
        let result = run(Some(2), "", stderr);
        assert!(
            no_verdict(command, &result, &host())
                .unwrap_or_else(|| panic!("{command}: {stderr}"))
                .contains(expected),
            "{command}: {stderr}"
        );
    }
}

#[path = "evidence_423_shell_tests.rs"]
mod evidence_423;
