use crate::write_boundary::landlock::LandlockSandbox;
use std::process::Command;

/// Install optional seals. Rule construction is handled separately by the caller.
pub fn install_operator_seals(command: &mut Command, active: bool) -> super::LaunchReport {
    let prepared = LandlockSandbox::probe_seals().and_then(|()| super::config::prepare());
    match prepared {
        Ok((paths, mut notes)) => {
            let result = LandlockSandbox::build_sealed(&paths);
            if result.is_err() {
                notes.extend(
                    paths
                        .iter()
                        .map(|path| format!("{} not sealed", path.display())),
                );
            }
            install_prepared_seals(command, result, active, &notes)
        }
        Err(reason) => {
            let mut paths = super::config::write_paths();
            paths.extend(super::operator_paths());
            paths.push(crate::credential_paths::operator_grant_root());
            paths.sort();
            paths.dedup();
            let notes: Vec<_> = paths
                .iter()
                .map(|path| format!("{} not sealed", path.display()))
                .collect();
            install_prepared_seals(command, Err(reason), active, &notes)
        }
    }
}

/// Install a prepared optional boundary, retaining failure diagnostics without refusing.
pub fn install_prepared_seals(
    command: &mut Command,
    result: Result<LandlockSandbox, String>,
    active: bool,
    notes: &[String],
) -> super::LaunchReport {
    let mut seals = None;
    let state = match result {
        Ok(boundary) => {
            seals = boundary.install_with_report(command);
            if active {
                "optional seals (child outcome below)".to_owned()
            } else {
                super::INACTIVE_MOUNT_WRITES.to_owned()
            }
        }
        Err(reason) => format!("not-contained: all optional seals absent ({reason})"),
    };
    super::LaunchReport {
        prepared: if notes.is_empty() {
            state
        } else {
            format!("{state}; preparation: {}", notes.join("; "))
        },
        seals,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn failed_optional_seals_allow_execution_for_both_rule_states() {
        for active in [false, true] {
            for reason in [
                "unshare, errno 1",
                "bind, errno 13",
                "recursive-readonly, errno 38",
            ] {
                let mut command = crate::spawn::command("/bin/true");
                let report =
                    super::install_prepared_seals(&mut command, Err(reason.into()), active, &[]);
                assert!(report.contains(reason), "{report}");
                assert!(report.contains("all optional seals absent"));
                assert!(command.status().unwrap().success());
                assert!(
                    crate::read_boundary::observation_note(format!("writes: {report}")).is_some()
                );
            }
        }
    }
}

#[cfg(test)]
#[test]
fn failed_seals_preserve_an_independent_rule_layer() {
    if crate::write_boundary::landlock::probe().is_err() {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI requires a usable rule layer"
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let protected = root.path().join("protected");
    std::fs::create_dir(&protected).unwrap();
    let boundary = LandlockSandbox::build(&[protected.clone()], &[], &[]).unwrap();
    let mut command = crate::spawn::command("/bin/sh");
    let report = install_prepared_seals(&mut command, Err("unshare, errno 1".into()), true, &[]);
    boundary.install_std(&mut command);
    let output = command
        .args([
            "-c",
            "printf forbidden > \"$1/new\"; printf executed",
            "probe",
        ])
        .arg(&protected)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"executed");
    assert!(!protected.join("new").exists());
    assert!(report.contains("unshare, errno 1"));
}

#[cfg(test)]
#[test]
fn installed_seals_without_rules_report_the_host_process_bypass() {
    let mut command = crate::spawn::command("/bin/true");
    let report =
        install_prepared_seals(&mut command, LandlockSandbox::build_sealed(&[]), false, &[]);
    assert!(command.status().unwrap().success());
    assert!(report.contains(super::INACTIVE_MOUNT_WRITES), "{report}");
    assert!(report.contains("applied seals:"), "{report}");
}
