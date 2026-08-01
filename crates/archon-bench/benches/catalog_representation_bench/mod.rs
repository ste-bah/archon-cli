use super::*;

pub(super) mod checks;

type BenchGroup<'a> = criterion::BenchmarkGroup<'a, criterion::measurement::WallTime>;

pub(super) fn bench_catalog_representations(criterion: &mut Criterion) {
    for size in FIXTURE_SIZES {
        let fixture = Fixture::new(size);
        bench_complete_publication(criterion, size, &fixture);
        bench_exact_get(criterion, size, &fixture);
        bench_highest_version_lookup(criterion, size, &fixture);
        bench_tag_capability_read(criterion, size, &fixture);
    }
}

fn bench_complete_publication(criterion: &mut Criterion, size: usize, fixture: &Fixture) {
    let mut group = criterion.benchmark_group("catalog_representation/complete_publication");
    bench_dash_publication(&mut group, size, fixture);
    bench_standard_publication(&mut group, size, fixture);
    group.finish();
}

fn bench_dash_publication(group: &mut BenchGroup<'_>, size: usize, fixture: &Fixture) {
    group.bench_with_input(
        BenchmarkId::new("dashmap_clone_plus_arcswap_store", size),
        fixture,
        |bench, fixture| {
            let target = ArcSwap::from_pointee(CatalogSnapshot::default());
            bench.iter(|| {
                let prepared = Arc::new(black_box(fixture.dash.clone()));
                target.store(black_box(prepared));
                black_box(target.load_full());
            });
        },
    );
}

fn bench_standard_publication(group: &mut BenchGroup<'_>, size: usize, fixture: &Fixture) {
    group.bench_with_input(
        BenchmarkId::new("standard_hashmap_clone_plus_arcswap_store", size),
        fixture,
        |bench, fixture| {
            let target = ArcSwap::from_pointee(ImmutableCatalogSnapshot::default());
            bench.iter(|| {
                let prepared = Arc::new(black_box(fixture.standard.clone()));
                target.store(black_box(prepared));
                black_box(target.load_full());
            });
        },
    );
}

fn bench_exact_get(criterion: &mut Criterion, size: usize, fixture: &Fixture) {
    let mut group = criterion.benchmark_group("catalog_representation/exact_get");
    bench_dash_exact_get(&mut group, size, fixture);
    bench_standard_exact_get(&mut group, size, fixture);
    group.finish();
}

fn bench_dash_exact_get(group: &mut BenchGroup<'_>, size: usize, fixture: &Fixture) {
    group.bench_with_input(
        BenchmarkId::new("dashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| {
                let metadata = fixture.dash_reads.exact_get(&fixture.exact_key);
                black_box(metadata_checksum(&metadata));
            })
        },
    );
}

fn bench_standard_exact_get(group: &mut BenchGroup<'_>, size: usize, fixture: &Fixture) {
    group.bench_with_input(
        BenchmarkId::new("standard_hashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| {
                let metadata = fixture.standard_reads.exact_get(&fixture.exact_key);
                black_box(metadata_checksum(&metadata));
            })
        },
    );
}

fn bench_highest_version_lookup(criterion: &mut Criterion, size: usize, fixture: &Fixture) {
    let mut group = criterion.benchmark_group("catalog_representation/highest_version_resolution");
    bench_dash_highest_version(&mut group, size, fixture);
    bench_standard_highest_version(&mut group, size, fixture);
    group.finish();
}

fn bench_dash_highest_version(group: &mut BenchGroup<'_>, size: usize, fixture: &Fixture) {
    group.bench_with_input(
        BenchmarkId::new("dashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| {
                let metadata = fixture.dash_reads.highest_version(&fixture.lookup_name);
                black_box(metadata_checksum(&metadata));
            })
        },
    );
}

fn bench_standard_highest_version(group: &mut BenchGroup<'_>, size: usize, fixture: &Fixture) {
    group.bench_with_input(
        BenchmarkId::new("standard_hashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| {
                let metadata = fixture.standard_reads.highest_version(&fixture.lookup_name);
                black_box(metadata_checksum(&metadata));
            })
        },
    );
}

fn bench_tag_capability_read(criterion: &mut Criterion, size: usize, fixture: &Fixture) {
    let mut group = criterion.benchmark_group("catalog_representation/tag_capability_indexed_read");
    bench_dash_indexed_read(&mut group, size, fixture);
    bench_standard_indexed_read(&mut group, size, fixture);
    group.finish();
}

fn bench_dash_indexed_read(group: &mut BenchGroup<'_>, size: usize, fixture: &Fixture) {
    group.bench_with_input(
        BenchmarkId::new("dashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| {
                let results = fixture
                    .dash_reads
                    .indexed_read(&fixture.tag, &fixture.capability);
                black_box(results_checksum(&results));
            })
        },
    );
}

fn bench_standard_indexed_read(group: &mut BenchGroup<'_>, size: usize, fixture: &Fixture) {
    group.bench_with_input(
        BenchmarkId::new("standard_hashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| {
                let results = fixture
                    .standard_reads
                    .indexed_read(&fixture.tag, &fixture.capability);
                black_box(results_checksum(&results));
            })
        },
    );
}
