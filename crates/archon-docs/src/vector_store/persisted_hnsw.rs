use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Sender, SyncSender};
use std::sync::{Mutex, OnceLock, mpsc};

use anyhow::{Context, Result};
use hnsw_rs::prelude::{DistCosine, HnswIo};

use super::{HnswManifest, HnswSearchHit};

pub(super) fn search(
    hnsw_dir: PathBuf,
    manifest: HnswManifest,
    load_chunk_ids: impl FnOnce() -> Result<HashMap<usize, String>>,
    query: Vec<f32>,
    top_k: usize,
    ef: usize,
) -> Result<Vec<HnswSearchHit>> {
    let identity = manifest_identity(&hnsw_dir, &manifest);
    let cache = {
        let mut caches = cache().lock().unwrap_or_else(|error| error.into_inner());
        if !caches.contains_key(&identity) {
            let loaded = Cache::load(hnsw_dir, manifest, load_chunk_ids()?)?;
            caches.insert(identity.clone(), loaded);
        }
        caches
            .get(&identity)
            .expect("persisted HNSW cache initialized")
            .clone()
    };
    cache.search(query, top_k, ef)
}

#[cfg(test)]
pub(super) fn clear() {
    let caches = {
        let mut cache = cache().lock().unwrap_or_else(|error| error.into_inner());
        std::mem::take(&mut *cache)
    };
    drop(caches);
}

pub(super) fn clear_dir(hnsw_dir: &std::path::Path) {
    let prefix = format!("{}:", hnsw_dir.display());
    let removed = {
        let mut cache = cache().lock().unwrap_or_else(|error| error.into_inner());
        let identities: Vec<_> = cache
            .keys()
            .filter(|identity| identity.starts_with(&prefix))
            .cloned()
            .collect();
        identities
            .into_iter()
            .filter_map(|identity| cache.remove(&identity))
            .collect::<Vec<_>>()
    };
    drop(removed);
}

#[cfg(test)]
pub(super) fn load_count() -> usize {
    LOADS.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
pub(super) fn cache_present() -> bool {
    !cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .is_empty()
}

#[cfg(test)]
pub(super) fn worker_count() -> usize {
    WORKERS.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone)]
struct Cache {
    requests: Sender<Request>,
}

struct Request {
    query: Vec<f32>,
    top_k: usize,
    ef: usize,
    response: Sender<Result<Vec<HnswSearchHit>>>,
}

impl Cache {
    fn load(
        hnsw_dir: PathBuf,
        manifest: HnswManifest,
        chunk_ids: HashMap<usize, String>,
    ) -> Result<Self> {
        let (requests, receiver) = mpsc::channel();
        let (ready, ready_receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("archon-persisted-hnsw".into())
            .spawn(move || worker(hnsw_dir, manifest, chunk_ids, receiver, ready))
            .context("start persisted HNSW cache worker")?;
        ready_receiver
            .recv()
            .context("wait for persisted HNSW cache worker")??;
        Ok(Self { requests })
    }

    fn search(&self, query: Vec<f32>, top_k: usize, ef: usize) -> Result<Vec<HnswSearchHit>> {
        let (response, receiver) = mpsc::channel();
        self.requests
            .send(Request {
                query,
                top_k,
                ef,
                response,
            })
            .context("send persisted HNSW search")?;
        receiver
            .recv()
            .context("receive persisted HNSW search result")?
    }
}

fn worker(
    hnsw_dir: PathBuf,
    manifest: HnswManifest,
    chunk_ids: HashMap<usize, String>,
    requests: mpsc::Receiver<Request>,
    ready: SyncSender<Result<()>>,
) {
    let panic_ready = ready.clone();
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        worker_loaded(hnsw_dir, manifest, chunk_ids, requests, ready)
    }))
    .is_err()
    {
        let _ = panic_ready.send(Err(anyhow::anyhow!(
            "load persisted HNSW snapshot panicked"
        )));
    }
}

fn worker_loaded(
    hnsw_dir: PathBuf,
    manifest: HnswManifest,
    chunk_ids: HashMap<usize, String>,
    requests: mpsc::Receiver<Request>,
    ready: SyncSender<Result<()>>,
) {
    let mut reloader = HnswIo::new(&hnsw_dir, &manifest.dump_basename);
    let hnsw = match reloader
        .load_hnsw::<f32, DistCosine>()
        .context("load persisted HNSW snapshot")
    {
        Ok(hnsw) => hnsw,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    #[cfg(test)]
    {
        LOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        WORKERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(test)]
    let _worker = WorkerGuard;
    if ready.send(Ok(())).is_err() {
        return;
    }
    for request in requests {
        let hits = search_hits(&hnsw, &chunk_ids, request.query, request.top_k, request.ef);
        let _ = request.response.send(hits);
    }
}

fn search_hits(
    hnsw: &hnsw_rs::prelude::Hnsw<'_, f32, DistCosine>,
    chunk_ids: &HashMap<usize, String>,
    query: Vec<f32>,
    top_k: usize,
    ef: usize,
) -> Result<Vec<HnswSearchHit>> {
    if top_k == 0 {
        return Ok(Vec::new());
    }
    hnsw.search(&query, top_k, ef.max(top_k))
        .into_iter()
        .map(|hit| {
            let origin_id = hit.get_origin_id();
            let chunk_id = chunk_ids
                .get(&origin_id)
                .with_context(|| format!("persisted HNSW hit {origin_id} is missing chunk ID"))?;
            Ok(HnswSearchHit {
                chunk_id: chunk_id.clone(),
                distance: hit.get_distance(),
            })
        })
        .collect()
}

fn manifest_identity(hnsw_dir: &std::path::Path, manifest: &HnswManifest) -> String {
    format!(
        "{}:{}:{}:{}:{:?}:{}",
        hnsw_dir.display(),
        manifest.provider,
        manifest.dimension,
        manifest.vector_count,
        manifest.provider_generation,
        manifest.dump_basename
    )
}

fn cache() -> &'static Mutex<HashMap<String, Cache>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Cache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
struct WorkerGuard;

#[cfg(test)]
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        WORKERS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
static LOADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
static WORKERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
