use super::*;

fn unavailable(kind: io::ErrorKind) {
    let bound = descriptor_ceiling_with(|| Err(io::Error::from(kind)))
        .expect("spawning must not require a mounted/readable descriptor directory");
    // Neither the soft nor hard rlimit bounds descriptors inherited before
    // either limit was lowered. The entire representable fd range does.
    assert_eq!(bound, libc::c_int::MAX);
}
#[test]
fn missing_procfs_does_not_abort_spawn_preparation() {
    unavailable(io::ErrorKind::NotFound);
}
#[test]
fn denied_procfs_does_not_abort_spawn_preparation() {
    unavailable(io::ErrorKind::PermissionDenied);
}
#[test]
fn failed_procfs_enumeration_still_covers_inherited_high_fds() {
    unavailable(io::ErrorKind::InvalidData);
}
