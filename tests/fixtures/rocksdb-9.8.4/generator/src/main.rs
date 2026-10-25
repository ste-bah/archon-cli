use std::path::Path;

use cozo::{DbInstance, ScriptMutability};
use rust_rocksdb::{Options, WriteBatch, DB};

fn key3(prefix: &str, provider: &str, value: &str) -> Vec<u8> {
    format!("{prefix}/{provider}/{value}").into_bytes()
}

fn vector_bytes(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4 + vector.len() * 4);
    bytes.extend_from_slice(&(vector.len() as u32).to_le_bytes());
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn hnsw_id(chunk: &str) -> usize {
    let hash = blake3::hash(chunk.as_bytes());
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&hash.as_bytes()[..8]);
    u64::from_le_bytes(bytes) as usize
}

fn make_vector_fixture(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    let mut options = Options::default();
    options.create_if_missing(true);
    options.set_max_open_files(256);
    options.increase_parallelism(
        std::thread::available_parallelism()
            .map(|count| count.get() as i32)
            .unwrap_or(2),
    );
    options.set_stats_dump_period_sec(0);
    options.set_stats_persist_period_sec(0);
    let db = DB::open(&options, path).unwrap();
    let entries = [
        ("old-chunk-a", "old-hash-a", vec![0.25, -1.5, 3.0]),
        ("old-chunk-b", "old-hash-b", vec![2.0, 4.5]),
    ];
    let mut batch = WriteBatch::default();
    for (chunk, hash, vector) in entries {
        let bytes = vector_bytes(&vector);
        batch.put(key3("vec", "compat-provider", chunk), &bytes);
        batch.put(key3("cache", "compat-provider", hash), &bytes);
        batch.put(
            key3("id", "compat-provider", chunk),
            hnsw_id(chunk).to_be_bytes(),
        );
        batch.put(
            key3("rid", "compat-provider", &hnsw_id(chunk).to_string()),
            chunk.as_bytes(),
        );
    }
    batch.put(key3("rid", "compat-provider", "ready"), []);
    batch.put(b"generation/compat-provider", 2_u64.to_be_bytes());
    db.write(batch).unwrap();
    db.flush().unwrap();
}

fn make_cozo_fixture(path: &Path) {
    let path = path.to_string_lossy().into_owned();
    let db = DbInstance::new("newrocksdb", &path, "").unwrap();
    db.run_script(
        ":create compat_rows {name: String => score: Int}",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
        "?[name, score] <- [[\"Ada\", 17], [\"Lin\", 23]] :put compat_rows {name => score}",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
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

fn remove_rocksdb_logs(path: &Path) {
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            remove_rocksdb_logs(&entry.path());
        } else if entry.file_name().to_string_lossy().starts_with("LOG") {
            std::fs::remove_file(entry.path()).unwrap();
        }
    }
}

fn compact_cozo_fixture(path: &Path) {
    let path = path.to_string_lossy().into_owned();
    let db = DbInstance::new("newrocksdb", &path, "").unwrap();
    db.run_script("::compact", Default::default(), ScriptMutability::Mutable)
        .unwrap();
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for name in ["vector-store", "cozo-learning", "cozo-learning-wal"] {
        let path = root.join(name);
        if path.exists() {
            std::fs::remove_dir_all(path).unwrap();
        }
    }
    make_vector_fixture(&root.join("vector-store"));
    make_cozo_fixture(&root.join("cozo-learning"));
    copy_dir(&root.join("cozo-learning"), &root.join("cozo-learning-wal"));
    compact_cozo_fixture(&root.join("cozo-learning"));
    for name in ["vector-store", "cozo-learning", "cozo-learning-wal"] {
        remove_rocksdb_logs(&root.join(name));
    }
}
