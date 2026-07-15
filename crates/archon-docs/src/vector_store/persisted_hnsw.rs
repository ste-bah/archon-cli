use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};

use anyhow::{Context, Result};
use hnsw_rs::prelude::{DistCosine, HnswIo};

use super::{HnswManifest, HnswSearchHit};

pub(super) fn search(
    hnsw_dir: PathBuf,
    manifest: HnswManifest,
    chunk_ids: HashMap<usize, String>,
    query: Vec<f32>,
    top_k: usize,
    ef: usize,
) -> Result<Vec<HnswSearchHit>> {
    let identity = manifest_identity(&hnsw_dir, &manifest);
    with_cache(|cache| {
        if cache
            .as_ref()
            .is_none_or(|cache| cache.identity != identity)
        {
            *cache = Some(Cache::load(hnsw_dir, manifest, chunk_ids, identity)?);
        }
        cache
            .as_ref()
            .expect("persisted HNSW cache initialized")
            .search(query, top_k, ef)
    })
}

#[cfg(test)]
pub(super) fn load_count() -> usize {
    LOADS.load(std::sync::atomic::Ordering::Relaxed)
}

struct Cache {
    identity: String,
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
        identity: String,
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
        Ok(Self { identity, requests })
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
    requests: Receiver<Request>,
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
    requests: Receiver<Request>,
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
    LOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if ready.send(Ok(())).is_err() {
        return;
    }
    for request in requests {
        let hits = if request.top_k == 0 {
            Ok(Vec::new())
        } else {
            Ok(hnsw
                .search(&request.query, request.top_k, request.ef.max(request.top_k))
                .into_iter()
                .filter_map(|hit| {
                    chunk_ids
                        .get(&hit.get_origin_id())
                        .map(|chunk_id| HnswSearchHit {
                            chunk_id: chunk_id.clone(),
                            distance: hit.get_distance(),
                        })
                })
                .collect())
        };
        let _ = request.response.send(hits);
    }
}

fn manifest_identity(hnsw_dir: &std::path::Path, manifest: &HnswManifest) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        hnsw_dir.display(),
        manifest.provider,
        manifest.dimension,
        manifest.vector_count,
        manifest.dump_basename
    )
}

thread_local! {
    static CACHE: std::cell::RefCell<Option<Cache>> = const { std::cell::RefCell::new(None) };
}

fn with_cache<T>(f: impl FnOnce(&mut Option<Cache>) -> Result<T>) -> Result<T> {
    CACHE.with(|cache| f(&mut cache.borrow_mut()))
}

#[cfg(test)]
static LOADS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
