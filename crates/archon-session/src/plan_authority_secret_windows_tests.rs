use super::{fs, load_or_create_approval_secret, owner_only_sddl, windows_secret};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NATIVE_FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct NativeSecretFixture {
    directory: PathBuf,
    path: PathBuf,
}

impl Drop for NativeSecretFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn native_secret_fixture() -> NativeSecretFixture {
    let temp = std::env::temp_dir();
    assert!(
        temp.is_dir(),
        "Windows temp directory is unavailable: {temp:?}"
    );
    for _ in 0..100 {
        let sequence = NATIVE_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let directory = temp.join(format!(
            "archon-session-plan-secret-{}-{sequence}",
            std::process::id()
        ));
        if fs::create_dir(&directory).is_ok() {
            let directory = fs::canonicalize(directory).unwrap();
            assert!(directory.is_dir());
            return NativeSecretFixture {
                path: directory.join("approval.secret"),
                directory,
            };
        }
    }
    panic!("could not create a unique native Windows secret fixture");
}

#[test]
fn owner_only_sddl_is_protected_and_limits_access_to_owner_and_system() {
    let sddl = owner_only_sddl("S-1-5-21-100-200-300-400");
    assert_eq!(
        sddl,
        "O:S-1-5-21-100-200-300-400G:S-1-5-21-100-200-300-400D:P(A;;FA;;;S-1-5-21-100-200-300-400)(A;;FA;;;SY)"
    );
    assert!(sddl.contains("D:P"));
    assert!(!sddl.contains(";;;WD)"));
    assert!(!sddl.contains(";;;BA)"));
}

#[test]
fn native_owner_only_secret_roundtrips_and_validates_its_acl() {
    let fixture = native_secret_fixture();
    let first = load_or_create_approval_secret(&fixture.path).unwrap();
    assert!(fixture.path.is_file());
    let second = load_or_create_approval_secret(&fixture.path).unwrap();
    assert_eq!(first.len(), 32);
    assert_eq!(first, second);
    windows_secret::validate_owner_only_acl(&fixture.path).unwrap();
}

#[test]
fn native_acl_tampering_is_rejected() {
    let fixture = native_secret_fixture();
    load_or_create_approval_secret(&fixture.path).unwrap();
    assert!(fixture.path.is_file());
    windows_secret::replace_acl_for_test(&fixture.path, "D:(A;;FA;;;WD)").unwrap();
    assert_eq!(
        load_or_create_approval_secret(&fixture.path)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
}
