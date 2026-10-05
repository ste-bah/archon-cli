//! Issue 333 round 3 (the disk that filled): a listing's output is read
//! from pipes and capped, never kept on disk; what it writes to its own
//! directories is capped too; and its directory is removed however the
//! listing ends.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::tests::Bin;
use super::tests_333::{STALL, counting_tool, warm};
use super::unresolved_on_path;

const PLUGIN: &str = "#!/bin/sh\nexit 0\n";

/// A tool whose `--list` first records its HOME in `record`, then runs
/// `listing`; the site it is listed at (the host has `{name}-sub`).
fn recording(name: &str, record: &Path, listing: &str) -> Bin {
    let calls = record.with_file_name("calls");
    let script = counting_tool(
        name,
        &calls,
        &format!("echo \"$HOME\" > '{}'; {listing}", record.display()),
    );
    let plugin = format!("{name}-sub");
    let bin = Bin::with_host(
        Path::new("/bin/sh"),
        &[(name, &script)],
        &[(&plugin, PLUGIN)],
    );
    warm(&bin, &[name]);
    bin
}

/// The listing directory whose HOME `record` names.
fn listing_root(record: &Path) -> PathBuf {
    let home = std::fs::read_to_string(record).unwrap();
    let root = Path::new(home.trim()).parent().unwrap().to_path_buf();
    let name = root.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with(super::list::SCRATCH_PREFIX), "{root:?}");
    root
}

#[test]
fn an_endless_printer_is_capped_and_leaves_nothing_behind() {
    let records = tempfile::tempdir().unwrap();
    let record = records.path().join("home");
    let bin = recording("archon333flood", &record, "exec /usr/bin/yes");
    let started = Instant::now();
    let warned = bin.warned("archon333flood sub");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(
        warned.len() == 1 && warned[0].contains("output exceeded 1048576 bytes"),
        "{warned:?}"
    );
    assert!(!listing_root(&record).exists(), "its directory is gone");
}

#[test]
fn a_listing_that_fills_its_own_directories_is_stopped() {
    let records = tempfile::tempdir().unwrap();
    let record = records.path().join("home");
    // A bounded write, past the 16 MiB cap, then a wait.
    let fills = "/usr/bin/head -c 17000000 /dev/zero > \"$TMPDIR/fill\"; exec /bin/sleep 30";
    let bin = recording("archon333fill", &record, fills);
    let warned = bin.warned("archon333fill sub");
    assert!(
        warned.len() == 1 && warned[0].contains("wrote more than 16777216 bytes"),
        "{warned:?}"
    );
    assert!(!listing_root(&record).exists(), "its directory is gone");
}

#[test]
fn every_way_a_listing_ends_removes_its_directory() {
    let cases = [
        (
            "archon333done",
            "printf 'Commands:\\n    build    Build\\n'; exit 0",
            "not built into",
        ),
        (
            "archon333fails",
            "echo 'unknown option: --list' >&2; exit 129",
            "exited 129",
        ),
        ("archon333stuck", "exec /bin/sleep 30", "printed nothing"),
        (
            "archon333shut",
            "exec >&- 2>&-; exec /bin/sleep 30",
            "printed nothing",
        ),
    ];
    for (name, listing, said) in cases {
        let records = tempfile::tempdir().unwrap();
        let record = records.path().join("home");
        let bin = recording(name, &record, listing);
        let mut at = bin.context();
        at.list_stall = STALL;
        let warned = unresolved_on_path(&[&format!("{name} sub")], &at);
        assert!(
            warned[0].len() == 1 && warned[0][0].contains(said),
            "{name}: {warned:?}"
        );
        assert!(
            !listing_root(&record).exists(),
            "{name}: its directory is gone"
        );
    }
}
