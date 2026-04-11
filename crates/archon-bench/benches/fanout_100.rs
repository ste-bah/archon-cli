//! fanout_100 bench — NFR-SCALABILITY-001 (100 workers reach RUNNING <1 s).
//! Phase-5 fanout task populates this body.

use criterion::{criterion_group, criterion_main, Criterion};

fn bench_fanout_100_stub(c: &mut Criterion) {
    c.bench_function("fanout_100_stub", |b| b.iter(|| {}));
}

criterion_group!(benches, bench_fanout_100_stub);
criterion_main!(benches);
