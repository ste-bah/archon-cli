//! Tests for the execution world's filesystem seam (#201 Phase 1).

use super::*;
use std::collections::BTreeSet;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "archon-fs-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[tokio::test]
async fn local_fs_round_trips_bytes() {
    let dir = temp_dir("round-trip");
    let file = dir.join("probe.txt");
    let fs = LocalFs;

    fs.write(&file, b"written through the seam")
        .await
        .expect("write");

    assert_eq!(
        fs.read(&file).await.expect("read"),
        b"written through the seam"
    );
    assert_eq!(
        fs.read_to_string(&file).await.expect("read_to_string"),
        "written through the seam"
    );
}

#[tokio::test]
async fn read_to_string_rejects_non_utf8_rather_than_lossily_decoding() {
    let dir = temp_dir("non-utf8");
    let file = dir.join("binary.bin");
    let fs = LocalFs;
    fs.write(&file, &[0xff, 0xfe, 0x00]).await.expect("write");

    let error = fs.read_to_string(&file).await.expect_err("invalid utf-8");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[tokio::test]
async fn create_dir_all_makes_missing_parents() {
    let dir = temp_dir("parents");
    let nested = dir.join("a").join("b").join("c");
    let fs = LocalFs;

    fs.create_dir_all(&nested).await.expect("create_dir_all");

    assert!(fs.exists(&nested).await);
    assert!(fs.metadata(&nested).await.expect("metadata").is_dir);
}

#[tokio::test]
async fn metadata_reports_length_and_a_modification_time() {
    let dir = temp_dir("metadata");
    let file = dir.join("sized.txt");
    let fs = LocalFs;
    fs.write(&file, b"12345").await.expect("write");

    let meta = fs.metadata(&file).await.expect("metadata");

    assert_eq!(meta.len, 5);
    assert!(!meta.is_dir);
    assert!(
        meta.modified_nanos.is_some(),
        "the host filesystem reports modification times"
    );
}

#[tokio::test]
async fn exists_is_false_for_a_path_that_is_not_there() {
    let dir = temp_dir("absent");
    let fs = LocalFs;

    assert!(!fs.exists(&dir.join("never-created")).await);
    assert!(fs.metadata(&dir.join("never-created")).await.is_err());
}

#[tokio::test]
async fn read_dir_lists_immediate_children_only() {
    let dir = temp_dir("listing");
    let fs = LocalFs;
    fs.write(&dir.join("one.txt"), b"1").await.expect("write");
    fs.write(&dir.join("two.txt"), b"2").await.expect("write");
    fs.create_dir_all(&dir.join("sub")).await.expect("subdir");
    fs.write(&dir.join("sub").join("buried.txt"), b"3")
        .await
        .expect("write");

    let names: BTreeSet<String> = fs
        .read_dir(&dir)
        .await
        .expect("read_dir")
        .into_iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
        .collect();

    assert_eq!(
        names,
        BTreeSet::from([
            "one.txt".to_string(),
            "two.txt".to_string(),
            "sub".to_string()
        ]),
        "read_dir descends no further than one level"
    );
}

#[tokio::test]
async fn remove_file_deletes_it() {
    let dir = temp_dir("remove");
    let file = dir.join("doomed.txt");
    let fs = LocalFs;
    fs.write(&file, b"x").await.expect("write");

    fs.remove_file(&file).await.expect("remove_file");

    assert!(!fs.exists(&file).await);
}

#[tokio::test]
async fn version_is_absent_for_a_missing_path() {
    let dir = temp_dir("version-absent");
    let fs = LocalFs;

    assert!(fs.version(&dir.join("nothing-here")).await.is_none());
}

#[tokio::test]
async fn version_changes_when_the_bytes_change() {
    let dir = temp_dir("version-change");
    let file = dir.join("edited.txt");
    let fs = LocalFs;
    fs.write(&file, b"before").await.expect("write");
    let before = fs.version(&file).await.expect("version before");

    fs.write(&file, b"after the edit, a different length")
        .await
        .expect("rewrite");
    let after = fs.version(&file).await.expect("version after");

    assert_ne!(before, after);
}

#[tokio::test]
async fn version_is_stable_when_nothing_happens() {
    let dir = temp_dir("version-stable");
    let file = dir.join("untouched.txt");
    let fs = LocalFs;
    fs.write(&file, b"steady").await.expect("write");

    let first = fs.version(&file).await.expect("first");
    let second = fs.version(&file).await.expect("second");

    assert_eq!(
        first, second,
        "re-reading an unchanged file must not invalidate an observation"
    );
}

#[test]
fn a_world_without_modification_times_still_produces_a_token() {
    let with_time = FileVersion::from_parts(10, Some(1_234));
    let without_time = FileVersion::from_parts(10, None);

    assert_ne!(
        with_time, without_time,
        "an unknown time is recorded, not silently treated as the epoch"
    );
    assert_eq!(
        without_time,
        FileVersion::from_parts(10, None),
        "the degraded token is at least stable, so it compares equal to itself"
    );
    assert_ne!(
        without_time,
        FileVersion::from_parts(11, None),
        "length alone still distinguishes a resized file"
    );
}

#[tokio::test]
async fn local_fs_is_the_default_world() {
    let dir = temp_dir("default-world");
    let file = dir.join("default.txt");

    let fs = local_fs();
    fs.write(&file, b"via the shared handle")
        .await
        .expect("write");

    assert_eq!(
        std::fs::read(&file).expect("host read"),
        b"via the shared handle",
        "the default world is the host filesystem, byte for byte"
    );
}
