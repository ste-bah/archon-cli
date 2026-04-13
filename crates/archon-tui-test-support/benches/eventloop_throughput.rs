use archon_tui_test_support::metrics::MetricsRecorder;
use archon_tui_test_support::mock_agent::{
    spawn_n_mock_agents, EventScript, MockAgent, MockEventKind,
};
use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use std::time::Instant;
use tokio::sync::mpsc;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("rt")
}

fn unbounded_single_producer_single_consumer(c: &mut Criterion) {
    let mut group = c.benchmark_group("eventloop_throughput");
    group.throughput(Throughput::Elements(10_000));
    group.bench_function("unbounded_single_producer_single_consumer", |b| {
        let rt = rt();
        b.iter(|| {
            rt.block_on(async {
                let (tx, mut rx) = mpsc::unbounded_channel::<MockEventKind>();
                let script = EventScript::new()
                    .burst_of(10_000, MockEventKind::MessageDelta("e".into()));
                let agent = MockAgent::new("bench", script);
                let producer = tokio::spawn(async move { agent.run(tx).await });
                let mut recorder = MetricsRecorder::new();
                let batch_ts = Instant::now();
                let mut received = 0usize;
                while let Some(_ev) = rx.recv().await {
                    received += 1;
                }
                recorder.observe_drain(received, batch_ts);
                let _ = producer.await;
                criterion::black_box(recorder.snapshot());
            });
        });
    });
    group.finish();
}

fn unbounded_100_producers_single_consumer(c: &mut Criterion) {
    let mut group = c.benchmark_group("eventloop_throughput");
    group.throughput(Throughput::Elements(100_000));
    group.bench_function("unbounded_100_producers_single_consumer", |b| {
        let rt = rt();
        b.iter(|| {
            rt.block_on(async {
                let (tx, mut rx) = mpsc::unbounded_channel::<MockEventKind>();
                let handles = spawn_n_mock_agents(100, 1000, tx);
                let mut recorder = MetricsRecorder::new();
                let batch_ts = Instant::now();
                let mut received = 0usize;
                while let Some(_ev) = rx.recv().await {
                    received += 1;
                    if received == 100_000 {
                        break;
                    }
                }
                recorder.observe_drain(received, batch_ts);
                for h in handles {
                    let _ = h.await;
                }
                criterion::black_box(recorder.snapshot());
            });
        });
    });
    group.finish();
}

fn p95_event_latency_under_10k_eps(c: &mut Criterion) {
    let mut group = c.benchmark_group("eventloop_throughput");
    group.throughput(Throughput::Elements(10_000));
    group.bench_function("p95_event_latency_under_10k_eps", |b| {
        let rt = rt();
        b.iter(|| {
            rt.block_on(async {
                let (tx, mut rx) = mpsc::unbounded_channel::<MockEventKind>();
                let script = EventScript::new()
                    .burst_of(10_000, MockEventKind::MessageDelta("l".into()));
                let agent = MockAgent::new("latency", script);
                let send_ts = Instant::now();
                let producer = tokio::spawn(async move { agent.run(tx).await });
                let mut recorder = MetricsRecorder::new();
                while let Some(_ev) = rx.recv().await {
                    recorder.observe_drain(1, send_ts);
                }
                let _ = producer.await;
                let snap = recorder.snapshot();
                criterion::black_box(snap.p95_us);
            });
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    unbounded_single_producer_single_consumer,
    unbounded_100_producers_single_consumer,
    p95_event_latency_under_10k_eps
);
criterion_main!(benches);
