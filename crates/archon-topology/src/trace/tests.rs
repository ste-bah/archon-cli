use std::sync::Arc;
use std::thread;

use super::*;
use crate::ir::{GraphOrigin, NodeRole, TaskNode};

fn temp_dir() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "archon-topology-trace-{}-{:?}",
        std::process::id(),
        thread::current().id()
    ));
    let unique = base.join(format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&unique).unwrap();
    unique
}

fn record(kind: TraceKind, node: &str) -> TraceRecord {
    TraceRecord::new("2026-08-02T00:00:00Z", "graph-1", kind).with_node(node)
}

#[test]
fn append_and_read_round_trip() {
    let dir = temp_dir();
    let writer = TraceWriter::create(&dir).unwrap();

    writer.append(&record(TraceKind::NodeStarted, "a")).unwrap();
    writer
        .append(&record(TraceKind::ToolAttempt, "a").with_tool("Bash"))
        .unwrap();

    let readout = read_trace(writer.path()).unwrap();
    assert_eq!(readout.records.len(), 2);
    assert_eq!(readout.malformed_lines, 0);
    assert!(!readout.truncated_tail);
    assert_eq!(readout.records[1].tool.as_deref(), Some("Bash"));
}

#[test]
fn reading_a_missing_trace_is_empty_not_an_error() {
    let dir = temp_dir();
    let readout = read_trace(&dir.join("absent.jsonl")).unwrap();
    assert!(readout.is_empty());
    assert!(!readout.truncated_tail);
}

#[test]
fn a_truncated_trace_reads_without_error_and_drops_the_fragment() {
    let dir = temp_dir();
    let path = dir.join(TRACE_FILE);
    let writer = TraceWriter::at_path(&path);
    writer.append(&record(TraceKind::NodeStarted, "a")).unwrap();
    writer
        .append(&record(TraceKind::NodeFinished, "a"))
        .unwrap();

    // Simulate a process dying mid-append: a complete prefix plus a fragment
    // with no terminator.
    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(br#"{"ts":"2026-08-02T00:00:00Z","graph_id":"graph-1","kind":"node_st"#)
        .unwrap();
    drop(file);

    let readout = read_trace(&path).unwrap();
    assert_eq!(readout.records.len(), 2, "complete lines must survive");
    assert!(readout.truncated_tail, "the fragment must be flagged");
    assert_eq!(readout.malformed_lines, 0, "a fragment is not malformed");
}

#[test]
fn a_trace_that_is_only_a_fragment_yields_nothing() {
    let dir = temp_dir();
    let path = dir.join(TRACE_FILE);
    fs::write(&path, br#"{"ts":"2026"#).unwrap();

    let readout = read_trace(&path).unwrap();
    assert!(readout.is_empty());
    assert!(readout.truncated_tail);
}

#[test]
fn a_malformed_complete_line_is_counted_not_fatal() {
    let dir = temp_dir();
    let path = dir.join(TRACE_FILE);
    let writer = TraceWriter::at_path(&path);
    writer.append(&record(TraceKind::NodeStarted, "a")).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{not json at all}\n")
        .unwrap();
    writer
        .append(&record(TraceKind::NodeFinished, "a"))
        .unwrap();

    let readout = read_trace(&path).unwrap();
    assert_eq!(readout.records.len(), 2);
    assert_eq!(readout.malformed_lines, 1);
}

#[test]
fn an_unknown_kind_parses_rather_than_failing_the_record() {
    let dir = temp_dir();
    let path = dir.join(TRACE_FILE);
    fs::write(
        &path,
        b"{\"ts\":\"2026-08-02T00:00:00Z\",\"graph_id\":\"g\",\"node_id\":\"a\",\
          \"kind\":\"a_kind_from_the_future\"}\n",
    )
    .unwrap();

    let readout = read_trace(&path).unwrap();
    assert_eq!(readout.records.len(), 1);
    assert_eq!(readout.records[0].kind, TraceKind::Unknown);
    assert_eq!(readout.records[0].node_id, "a");
    assert_eq!(readout.malformed_lines, 0);
}

#[test]
fn n_threads_appending_produce_n_well_formed_lines() {
    const THREADS: usize = 12;
    const PER_THREAD: usize = 60;

    let dir = temp_dir();
    let writer = Arc::new(TraceWriter::create(&dir).unwrap());

    let handles: Vec<_> = (0..THREADS)
        .map(|worker| {
            let writer = Arc::clone(&writer);
            thread::spawn(move || {
                for index in 0..PER_THREAD {
                    let record =
                        TraceRecord::new("2026-08-02T00:00:00Z", "graph-1", TraceKind::ToolAttempt)
                            .with_node(format!("node-{worker}"))
                            .with_tool("Bash")
                            .with_attempt(u32::try_from(index).unwrap())
                            // Pad so a torn write would be obvious rather than lucky.
                            .with_detail("x".repeat(200));
                    writer.append(&record).unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    let readout = read_trace(writer.path()).unwrap();
    assert_eq!(
        readout.records.len(),
        THREADS * PER_THREAD,
        "every append must land as exactly one line"
    );
    assert_eq!(readout.malformed_lines, 0, "no line may be torn");
    assert!(!readout.truncated_tail);

    // Every worker's full contribution must be present, so nothing was
    // overwritten by a racing append either.
    for worker in 0..THREADS {
        let count = readout
            .records
            .iter()
            .filter(|record| record.node_id == format!("node-{worker}"))
            .count();
        assert_eq!(count, PER_THREAD, "worker {worker} lost records");
    }
}

#[test]
fn a_read_concurrent_with_appends_never_observes_a_partial_line() {
    const WRITERS: usize = 6;
    const PER_WRITER: usize = 250;

    let dir = temp_dir();
    let writer = Arc::new(TraceWriter::create(&dir).unwrap());
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let handles: Vec<_> = (0..WRITERS)
        .map(|worker| {
            let writer = Arc::clone(&writer);
            thread::spawn(move || {
                for index in 0..PER_WRITER {
                    writer
                        .append(
                            &TraceRecord::new(
                                "2026-08-02T00:00:00Z",
                                "graph-1",
                                TraceKind::ToolAttempt,
                            )
                            .with_node(format!("node-{worker}"))
                            .with_attempt(u32::try_from(index).unwrap())
                            .with_detail("y".repeat(300)),
                        )
                        .unwrap();
                }
            })
        })
        .collect();

    // Fold-shaped reader: runs repeatedly while writers are live. Every read
    // must yield only whole, parseable records.
    let reader_writer = Arc::clone(&writer);
    let reader_done = Arc::clone(&done);
    let reader = thread::spawn(move || {
        let mut reads = 0usize;
        let mut high_water = 0usize;
        while !reader_done.load(std::sync::atomic::Ordering::Relaxed) {
            let readout = read_trace(reader_writer.path()).unwrap();
            assert_eq!(
                readout.malformed_lines, 0,
                "a concurrent read observed a torn line"
            );
            high_water = high_water.max(readout.records.len());
            reads += 1;
        }
        (reads, high_water)
    });

    for handle in handles {
        handle.join().unwrap();
    }
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let (reads, _) = reader.join().unwrap();
    assert!(reads > 0, "the reader never ran");

    let final_readout = read_trace(writer.path()).unwrap();
    assert_eq!(final_readout.records.len(), WRITERS * PER_WRITER);
    assert_eq!(final_readout.malformed_lines, 0);
}

#[test]
fn an_oversized_record_sheds_optional_fields_rather_than_being_dropped() {
    let record = TraceRecord::new("2026-08-02T00:00:00Z", "graph-1", TraceKind::ToolAttempt)
        .with_node("a")
        .with_writes(
            (0..4000)
                .map(|i| WriteTarget::Path(format!("src/generated/file-{i}.rs")))
                .collect(),
        );

    let line = encode_record(&record).unwrap();
    assert!(
        line.len() <= MAX_RECORD_BYTES,
        "line was {} bytes",
        line.len()
    );
    assert!(line.ends_with('\n'));
    let parsed: TraceRecord = serde_json::from_str(line.trim_end()).unwrap();
    assert_eq!(parsed.node_id, "a", "attribution must survive shedding");
    assert!(parsed.writes.is_empty());
}

#[test]
fn detail_is_truncated_at_construction() {
    let record = TraceRecord::new("t", "g", TraceKind::ToolAttempt).with_detail("z".repeat(10_000));
    assert_eq!(record.detail.unwrap().chars().count(), MAX_DETAIL_CHARS);
}

#[test]
fn paths_resolve_under_the_project_archon_directory() {
    let paths = TopologyPaths::for_project(std::path::Path::new("/project"));
    assert!(
        paths
            .root()
            .ends_with(std::path::Path::new(".archon/topology"))
    );
    assert!(paths.trace_jsonl("g1").ends_with("g1/trace.jsonl"));
    assert!(paths.graph_json("g1").ends_with("g1/graph.json"));
    assert!(paths.ingested_marker("g1").ends_with("g1/ingested"));
}

#[test]
fn graph_ids_cannot_escape_the_topology_root() {
    // Separators are replaced, so the result is always a single component and
    // an embedded `..` cannot traverse.
    assert_eq!(sanitize_graph_id("../../etc/passwd"), "_.._.._etc_passwd");
    assert_eq!(sanitize_graph_id(".."), "unnamed");
    assert_eq!(sanitize_graph_id("."), "unnamed");
    assert_eq!(sanitize_graph_id(""), "unnamed");
    assert_eq!(sanitize_graph_id(".hidden"), "_.hidden");
    assert_eq!(sanitize_graph_id("wf-1a2b_ok.v2"), "wf-1a2b_ok.v2");
    assert_eq!(sanitize_graph_id("a/b\\c"), "a_b_c");
    for id in ["../../etc/passwd", "..", "", ".", "a/b\\c", "x:y*z"] {
        let sanitized = sanitize_graph_id(id);
        assert_eq!(
            std::path::Path::new(&sanitized).components().count(),
            1,
            "{id:?} sanitized to a multi-component path: {sanitized:?}"
        );
    }
}

#[test]
fn graph_round_trips_through_disk() {
    let dir = temp_dir();
    let paths = TopologyPaths::at_root(&dir);
    let mut graph = TaskGraph::new(
        "g1",
        GraphOrigin::Session {
            session_id: "s1".into(),
        },
    );
    graph.nodes.push(TaskNode::new("a", NodeRole::Work));

    assert!(paths.read_graph("g1").unwrap().is_none());
    paths.write_graph(&graph).unwrap();
    assert_eq!(paths.read_graph("g1").unwrap(), Some(graph));
}

#[test]
fn a_corrupt_graph_json_reads_as_absent_rather_than_erroring() {
    let dir = temp_dir();
    let paths = TopologyPaths::at_root(&dir);
    fs::create_dir_all(paths.graph_dir("g1")).unwrap();
    fs::write(paths.graph_json("g1"), b"{ half a gr").unwrap();

    assert!(paths.read_graph("g1").unwrap().is_none());
}

#[test]
fn the_ingested_marker_reports_and_persists() {
    let dir = temp_dir();
    let paths = TopologyPaths::at_root(&dir);
    assert!(!paths.is_ingested("g1"));
    paths.mark_ingested("g1", "fold-1").unwrap();
    assert!(paths.is_ingested("g1"));
    assert_eq!(
        fs::read_to_string(paths.ingested_marker("g1")).unwrap(),
        "fold-1"
    );
}

#[test]
fn listing_graph_ids_tolerates_a_missing_root() {
    let dir = temp_dir();
    let paths = TopologyPaths::at_root(dir.join("never-created"));
    assert!(paths.list_graph_ids().unwrap().is_empty());
}

#[test]
fn listing_graph_ids_returns_sorted_directories() {
    let dir = temp_dir();
    let paths = TopologyPaths::at_root(&dir);
    paths.writer("g2").unwrap();
    paths.writer("g1").unwrap();
    fs::write(dir.join("stray-file"), b"ignored").unwrap();

    assert_eq!(paths.list_graph_ids().unwrap(), vec!["g1", "g2"]);
}
