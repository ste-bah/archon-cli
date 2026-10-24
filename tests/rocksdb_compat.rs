use std::path::{Path, PathBuf};

use cozo::{DbInstance, ScriptMutability};

const FIXTURE_DIR: &str = "tests/fixtures/rocksdb-9.8.4";

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE_DIR)
        .join(name)
}

fn copy_dir(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn read_rows(db_path: &Path) -> Result<Vec<(String, i64)>, String> {
    let path_str = db_path.to_string_lossy().into_owned();
    // The bin's only open_db is src/bin/archon_learning_migrate.rs:146-149.
    let db = archon_cozo::GuardedDbInstance::new(
        DbInstance::new("newrocksdb", &path_str, "").map_err(|error| error.to_string())?,
        archon_cozo::CozoGuardConfig::for_db_path(db_path),
    );
    let rows = db
        .db()
        .run_script(
            "?[name, score] := *compat_rows{name, score}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|error| error.to_string())?;
    let mut actual = rows
        .rows
        .into_iter()
        .map(|row| {
            Ok((
                row[0]
                    .get_str()
                    .ok_or_else(|| "row name is not a string".to_owned())?
                    .to_owned(),
                row[1]
                    .get_int()
                    .ok_or_else(|| "row score is not an integer".to_owned())?,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    actual.sort();
    Ok(actual)
}

fn assert_fixture_rows(name: &str) {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("learning.db");
    copy_dir(&fixture_path(name), &db_path);
    assert_eq!(
        read_rows(&db_path).unwrap(),
        vec![("Ada".to_owned(), 17), ("Lin".to_owned(), 23)]
    );
}

#[test]
fn rocksdb_984_cozo_sst_fixture_opens_and_reads_rows() {
    assert_fixture_rows("cozo-learning");
}

#[test]
fn rocksdb_984_cozo_wal_fixture_opens_and_reads_rows() {
    assert_fixture_rows("cozo-learning-wal");
}

#[test]
fn rocksdb_984_cozo_fixture_rejects_corrupted_rows_file() {
    let fixture = fixture_path("cozo-learning");
    let data_dir = fixture.join("data");
    let files = std::fs::read_dir(&data_dir)
        .unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    let row_file = files
        .iter()
        .find(|path| path.extension().is_some_and(|extension| extension == "sst"))
        .or_else(|| {
            files
                .iter()
                .find(|path| path.extension().is_some_and(|extension| extension == "log"))
        })
        .unwrap();
    let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/corrupt-cozo");
    let _ = std::fs::remove_dir_all(&scratch);
    let db_path = scratch.join("learning.db");
    copy_dir(&fixture, &db_path);
    let corrupt_file = db_path.join("data").join(row_file.file_name().unwrap());
    let mut bytes = std::fs::read(&corrupt_file).unwrap();
    let offset = bytes.len() / 2;
    bytes[offset] ^= 0x01;
    std::fs::write(corrupt_file, bytes).unwrap();

    let result = read_rows(&db_path);
    std::fs::remove_dir_all(scratch).unwrap();
    assert!(
        result.as_ref().is_err()
            || result
                .as_ref()
                .is_ok_and(|rows| rows != &vec![("Ada".to_owned(), 17), ("Lin".to_owned(), 23)]),
        "corrupt Cozo rows must error or disappear; got {result:?}"
    );
}
