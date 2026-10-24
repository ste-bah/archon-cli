use super::*;

const FIXTURE_DIR: &str = "../../tests/fixtures/rocksdb-9.8.4/vector-store";

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIR)
}

fn copy_fixture(destination: &Path) {
    let source = fixture_path();
    for entry in std::fs::read_dir(&source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn copy_dir(source: &Path, destination: &Path) {
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn rocksdb_984_vector_fixture_reads_and_survives_current_write_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("vector-store");
    std::fs::create_dir(&db_path).unwrap();
    copy_fixture(&db_path);

    {
        let store = DocVectorStore::open(&db_path).unwrap();
        assert!(store.has_vector("compat-provider", "old-chunk-a").unwrap());
        assert!(store.has_vector("compat-provider", "old-chunk-b").unwrap());
        assert_eq!(
            store
                .cached_embedding("compat-provider", "old-hash-a")
                .unwrap(),
            Some(vec![0.25, -1.5, 3.0])
        );
        assert_eq!(
            store
                .cached_embedding("compat-provider", "old-hash-b")
                .unwrap(),
            Some(vec![2.0, 4.5])
        );
        let mut vectors = store
            .iter_records(Some("compat-provider"), None)
            .unwrap()
            .into_iter()
            .map(|record| (record.chunk_id, record.vector))
            .collect::<Vec<_>>();
        vectors.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(
            vectors,
            vec![
                ("old-chunk-a".to_owned(), vec![0.25, -1.5, 3.0]),
                ("old-chunk-b".to_owned(), vec![2.0, 4.5]),
            ]
        );
        assert_eq!(store.stats(Some("compat-provider")).unwrap().raw_vectors, 2);
        store
            .put_vectors(&[VectorWrite {
                chunk_id: "new-chunk",
                content_hash: "new-hash",
                provider: "compat-provider",
                embedding: &[7.0, 8.0, 9.0],
            }])
            .unwrap();
    }

    let reopened = DocVectorStore::open(&db_path).unwrap();
    assert_eq!(
        reopened
            .cached_embedding("compat-provider", "old-hash-a")
            .unwrap(),
        Some(vec![0.25, -1.5, 3.0])
    );
    assert_eq!(
        reopened
            .cached_embedding("compat-provider", "old-hash-b")
            .unwrap(),
        Some(vec![2.0, 4.5])
    );
    assert_eq!(
        reopened
            .cached_embedding("compat-provider", "new-hash")
            .unwrap(),
        Some(vec![7.0, 8.0, 9.0])
    );
    let mut vectors = reopened
        .iter_records(Some("compat-provider"), None)
        .unwrap()
        .into_iter()
        .map(|record| (record.chunk_id, record.vector))
        .collect::<Vec<_>>();
    vectors.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        vectors,
        vec![
            ("new-chunk".to_owned(), vec![7.0, 8.0, 9.0]),
            ("old-chunk-a".to_owned(), vec![0.25, -1.5, 3.0]),
            ("old-chunk-b".to_owned(), vec![2.0, 4.5]),
        ]
    );
    assert_eq!(
        reopened.stats(Some("compat-provider")).unwrap().raw_vectors,
        3
    );
}

#[test]
fn rocksdb_984_vector_fixture_rejects_a_corrupted_sst() {
    let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/corrupt-vector");
    let scratch = scratch.canonicalize().unwrap_or(scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    copy_fixture(&scratch);

    let sst = std::fs::read_dir(&scratch)
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "sst")
        })
        .unwrap()
        .path();
    let mut bytes = std::fs::read(&sst).unwrap();
    let offset = bytes.len() / 2;
    bytes[offset] ^= 0x01;
    std::fs::write(sst, bytes).unwrap();

    let error = DocVectorStore::open(&scratch)
        .err()
        .expect("corrupt SST must fail to open");
    let message = format!("{error:#}");
    std::fs::remove_dir_all(&scratch).unwrap();
    assert!(message.to_lowercase().contains("checksum"), "{message}");
}
