use std::path::{Path, PathBuf};

pub(crate) fn archon_binary() -> PathBuf {
    let binary = resolve_archon_binary();
    verify_binary_identity(&binary, env!("ARCHON_BUILD_FINGERPRINT"))
        .unwrap_or_else(|error| panic!("{error}"));
    binary
}

#[test]
fn launched_binary_exposes_its_build_identity() {
    let binary = resolve_archon_binary();
    let output = std::process::Command::new(&binary)
        .arg("--build-identity")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "build identity probe failed for {}: {}",
        binary.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        env!("ARCHON_BUILD_FINGERPRINT")
    );
}

#[test]
fn binary_with_a_different_build_identity_is_refused() {
    let mismatch = verify_binary_identity(&resolve_archon_binary(), "different-build").unwrap_err();
    assert!(
        mismatch.contains("binary build identity mismatch"),
        "{mismatch}"
    );
}

fn resolve_archon_binary() -> PathBuf {
    let binary = std::env::var_os("CARGO_BIN_EXE_archon")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(format!("archon{}", std::env::consts::EXE_SUFFIX))
        });
    assert!(
        binary.is_file(),
        "built archon executable missing: {}",
        binary.display()
    );
    binary
}

fn verify_binary_identity(binary: &Path, expected: &str) -> Result<(), String> {
    let output = std::process::Command::new(binary)
        .arg("--build-identity")
        .output()
        .map_err(|error| {
            format!(
                "cannot read build identity from {}: {error}",
                binary.display()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "cannot read build identity from {}: exited {}; stderr={}",
            binary.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if actual != expected {
        return Err(format!(
            "binary build identity mismatch: expected {}; received {} from {}",
            expected,
            actual,
            binary.display()
        ));
    }
    Ok(())
}
