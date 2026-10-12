use super::*;
fn actual_free() -> Arc<dyn Fn(&Path) -> io::Result<u64> + Send + Sync> {
    Arc::new(|path| fs2::available_space(path))
}

#[test]
fn rotation_and_concurrent_scopes_keep_every_record_once() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("transport.jsonl");
    let barrier = Arc::new(std::sync::Barrier::new(6));
    std::thread::scope(|threads| {
        for n in 0..6 {
            let scope =
                EvidenceScope::with_limits(path.clone(), &n.to_string(), 900, 0, actual_free())
                    .unwrap();
            let barrier = barrier.clone();
            threads.spawn(move || {
                barrier.wait();
                for index in 0..8 {
                    scope.record(json!({"kind":"sample","n":n,"i":index}));
                }
                scope.check().unwrap();
            });
        }
    });
    let raw = read_transport_evidence(&path).unwrap();
    let records: Vec<Value> = raw
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        records.iter().filter(|row| row["kind"] == "sample").count(),
        48
    );
    for n in 0..6 {
        for i in 0..8 {
            assert_eq!(
                records
                    .iter()
                    .filter(|row| row["n"] == n && row["i"] == i)
                    .count(),
                1
            );
        }
    }
    assert!(list_transport_segments(&path).unwrap().len() > 1);
}

#[test]
fn reader_lists_numeric_segments_before_active_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("transport.jsonl");
    std::fs::write(root.path().join("transport.000010.jsonl"), "10\n").unwrap();
    std::fs::write(root.path().join("transport.000002.jsonl"), "2\n").unwrap();
    std::fs::write(&path, "active\n").unwrap();
    assert_eq!(read_transport_evidence(&path).unwrap(), "2\n10\nactive\n");
}

#[test]
fn disk_low_prunes_oldest_and_records_prune_event() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("transport.jsonl");
    std::fs::write(root.path().join("transport.000001.jsonl"), "old1\n").unwrap();
    std::fs::write(root.path().join("transport.000002.jsonl"), "old2\n").unwrap();
    let probe_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let probe_counter = probe_count.clone();
    let probe = Arc::new(move |_: &Path| {
        let call = probe_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(if call == 0 { 1 } else { 25 })
    });
    let scope = EvidenceScope::with_limits(path.clone(), "call", 1, 20, probe).unwrap();
    scope.record(json!({"kind":"sample"}));
    scope.record(json!({"kind":"sample2"}));
    assert!(!root.path().join("transport.000001.jsonl").exists());
    assert!(root.path().join("transport.000002.jsonl").exists());
    assert!(
        read_transport_evidence(&path)
            .unwrap()
            .contains("evidence_pruned")
    );
}

#[test]
fn disk_low_without_segments_caps_once() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("transport.jsonl");
    let scope =
        EvidenceScope::with_limits(path.clone(), "call", 1, 20, Arc::new(|_| Ok(1))).unwrap();
    scope.record(json!({"kind":"one"}));
    scope.record(json!({"kind":"two"}));
    let another_scope =
        EvidenceScope::with_limits(path.clone(), "another", 1, 20, Arc::new(|_| Ok(1))).unwrap();
    another_scope.record(json!({"kind":"three"}));
    let raw = read_transport_evidence(&path).unwrap();
    assert_eq!(raw.matches("evidence_capped").count(), 1);
    assert!(raw.contains("disk_low"));
    assert!(raw.contains("free_bytes"));
}

type Probe = Arc<dyn Fn(&Path) -> io::Result<u64> + Send + Sync>;

fn switchable_probe(free: u64) -> (Arc<std::sync::atomic::AtomicU64>, Probe) {
    let value = Arc::new(std::sync::atomic::AtomicU64::new(free));
    let reader = value.clone();
    let probe: Probe =
        Arc::new(move |_: &Path| Ok(reader.load(std::sync::atomic::Ordering::SeqCst)));
    (value, probe)
}

fn kinds(path: &Path) -> Vec<String> {
    read_transport_evidence(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap()["kind"].to_string())
        .map(|kind| kind.trim_matches('"').to_string())
        .collect()
}

#[test]
fn disk_low_cap_drops_records_then_resumes_when_space_returns() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("transport.jsonl");
    let (free, probe) = switchable_probe(1);
    let scope = EvidenceScope::with_limits(path.clone(), "call", 1, 20, probe).unwrap();
    scope.record(json!({"kind":"one"}));
    scope.record(json!({"kind":"two"}));
    scope.record(json!({"kind":"three"}));
    assert_eq!(kinds(&path), ["one", "evidence_capped"]);
    free.store(25, std::sync::atomic::Ordering::SeqCst);
    scope.record(json!({"kind":"four"}));
    scope.record(json!({"kind":"five"}));
    let all = kinds(&path);
    assert_eq!(all.iter().filter(|k| *k == "evidence_capped").count(), 1);
    assert_eq!(all.iter().filter(|k| *k == "evidence_resumed").count(), 1);
    let resumed = all.iter().position(|k| k == "evidence_resumed").unwrap();
    let four = all.iter().position(|k| k == "four").unwrap();
    assert!(resumed < four);
    assert!(all.contains(&"five".to_string()));
    assert!(!all.contains(&"two".to_string()) && !all.contains(&"three".to_string()));
    let raw = read_transport_evidence(&path).unwrap();
    let line = raw
        .lines()
        .find(|l| l.contains("evidence_resumed"))
        .unwrap();
    let record: Value = serde_json::from_str(line).unwrap();
    assert_eq!(record["free_bytes"], 25);
    assert_eq!(record["floor_bytes"], 20);
    scope.check().unwrap();
}

#[test]
fn new_scope_on_disk_low_capped_tail_resumes_when_space_returns() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("transport.jsonl");
    let cap = json!({"kind":"evidence_capped","reason":"disk_low","free_bytes":1});
    std::fs::write(&path, format!("{}\n{cap}\n", json!({"kind":"old"}))).unwrap();
    let (free, probe) = switchable_probe(1);
    let scope = EvidenceScope::with_limits(path.clone(), "fresh", 1 << 20, 20, probe).unwrap();
    scope.record(json!({"kind":"dropped"}));
    assert_eq!(kinds(&path), ["old", "evidence_capped"]);
    free.store(25, std::sync::atomic::Ordering::SeqCst);
    scope.record(json!({"kind":"kept"}));
    scope.record(json!({"kind":"kept2"}));
    assert_eq!(
        kinds(&path),
        [
            "old",
            "evidence_capped",
            "evidence_resumed",
            "kept",
            "kept2"
        ]
    );
}
