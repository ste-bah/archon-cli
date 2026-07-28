use anyhow::{Context, Result};
use rust_rocksdb::DB;

const GENERATION_PREFIX: &str = "generation";

pub(super) fn current(db: &DB, provider: &str) -> Result<u64> {
    match db
        .get(key(provider))
        .context("read provider vector generation from RocksDB")?
    {
        Some(bytes) => decode(&bytes),
        None => Ok(0),
    }
}

pub(super) fn next(db: &DB, provider: &str) -> Result<u64> {
    current(db, provider)?
        .checked_add(1)
        .context("provider vector generation overflow")
}

pub(super) fn key(provider: &str) -> Vec<u8> {
    format!("{GENERATION_PREFIX}/{provider}").into_bytes()
}

pub(super) fn encode(generation: u64) -> [u8; std::mem::size_of::<u64>()] {
    generation.to_be_bytes()
}

fn decode(bytes: &[u8]) -> Result<u64> {
    anyhow::ensure!(
        bytes.len() == std::mem::size_of::<u64>(),
        "provider vector generation has invalid length"
    );
    Ok(u64::from_be_bytes(bytes.try_into()?))
}
