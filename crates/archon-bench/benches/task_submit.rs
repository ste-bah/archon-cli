//! task_submit bench — NFR-PERF-001 (<100 ms p95 submit path).
//! Phase-1 task_submit implementation task populates this body.

use criterion::{Criterion, criterion_group, criterion_main};

fn bench_task_submit_stub(c: &mut Criterion) {
    c.bench_function("task_submit_stub", |b| b.iter(|| {}));
}

criterion_group!(benches, bench_task_submit_stub);
criterion_main!(benches);
