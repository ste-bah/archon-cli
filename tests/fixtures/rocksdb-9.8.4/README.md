# RocksDB 9.8.4 compatibility fixtures

These small databases were generated on macOS arm64 with the checked-in helper crate in `generator/`. Its dependencies are pinned to `rust-rocksdb = 0.34.0` (`rust-librocksdb-sys 0.30.0+9.8.4`) and registry `cozo-ce = 0.7.13-alpha.3` with `default-features = false` and `storage-new-rocksdb, rayon`. This is the Cozo version and engine used before the RocksDB upgrade in commit `765304151`.

Generate them from the repository root with:

```sh
CARGO_TARGET_DIR=target/w RUST_TEST_THREADS=4 /private/tmp/cargo-slots/cargo-slot.sh run --locked --offline --manifest-path tests/fixtures/rocksdb-9.8.4/generator/Cargo.toml
```

`vector-store/` uses the default column family and the persisted key and value encodings from `crates/archon-docs/src/vector_store.rs` at the parent of `765304151`: two `vec/compat-provider/…` values and their `cache/`, `id/`, `rid/`, marker, and generation records. The known vectors are `[0.25, -1.5, 3.0]` and `[2.0, 4.5]`.

`cozo-learning/` is a `newrocksdb` database created with the same empty engine-options string used by `archon_learning_migrate::open_db`. Its `compat_rows` relation containing `Ada, 17` and `Lin, 23` is compacted into an SST. `cozo-learning-wal/` contains the same rows in the write-ahead log without an SST.

The fixture files are intended to be copied to a temporary directory before opening. Their OPTIONS files record the RocksDB writer version (`rocksdb_version=9.8.4`).
