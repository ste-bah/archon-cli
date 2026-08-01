use super::*;

pub(super) fn bench_catalog_representations(criterion: &mut Criterion) {
    for size in FIXTURE_SIZES {
        let fixture = Fixture::new(size);
        bench_clone_preparation(criterion, size, &fixture);
        bench_exact_get(criterion, size, &fixture);
        bench_highest_version_lookup(criterion, size, &fixture);
        bench_tag_capability_read(criterion, size, &fixture);
    }
}

fn bench_clone_preparation(criterion: &mut Criterion, size: usize, fixture: &Fixture) {
    let mut group = criterion.benchmark_group("catalog_representation/complete_publication");
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
    group.bench_with_input(
        BenchmarkId::new("standard_hashmap_clone_plus_arcswap_store", size),
        fixture,
        |bench, fixture| {
            let target = ArcSwap::from_pointee(StandardMapSnapshot::default());
            bench.iter(|| {
                let prepared = Arc::new(black_box(fixture.standard.clone()));
                target.store(black_box(prepared));
                black_box(target.load_full());
            });
        },
    );
    group.finish();
}

fn bench_exact_get(criterion: &mut Criterion, size: usize, fixture: &Fixture) {
    let mut group = criterion.benchmark_group("catalog_representation/exact_get");
    bench_exact_get_dash(&mut group, size, fixture);
    bench_exact_get_standard(&mut group, size, fixture);
    group.finish();
}

fn bench_exact_get_dash(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    size: usize,
    fixture: &Fixture,
) {
    group.bench_with_input(
        BenchmarkId::new("dashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| {
                black_box(
                    fixture
                        .dash
                        .entries
                        .get(&fixture.exact_key)
                        .map(|entry| metadata_checksum(entry.value()))
                        .expect("exact DashMap fixture entry"),
                )
            })
        },
    );
}

fn bench_exact_get_standard(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    size: usize,
    fixture: &Fixture,
) {
    group.bench_with_input(
        BenchmarkId::new("standard_hashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| {
                black_box(
                    fixture
                        .standard
                        .entries
                        .get(&fixture.exact_key)
                        .map(metadata_checksum)
                        .expect("exact standard-map fixture entry"),
                )
            })
        },
    );
}

fn bench_highest_version_lookup(criterion: &mut Criterion, size: usize, fixture: &Fixture) {
    let mut group =
        criterion.benchmark_group("catalog_representation/highest_version_index_lookup");
    bench_highest_version_dash(&mut group, size, fixture);
    bench_highest_version_standard(&mut group, size, fixture);
    group.finish();
}

fn bench_highest_version_dash(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    size: usize,
    fixture: &Fixture,
) {
    group.bench_with_input(
        BenchmarkId::new("dashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| black_box(highest_dash(&fixture.dash, &fixture.lookup_name)))
        },
    );
}

fn bench_highest_version_standard(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    size: usize,
    fixture: &Fixture,
) {
    group.bench_with_input(
        BenchmarkId::new("standard_hashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| black_box(highest_standard(&fixture.standard, &fixture.lookup_name)))
        },
    );
}

fn bench_tag_capability_read(criterion: &mut Criterion, size: usize, fixture: &Fixture) {
    let mut group = criterion.benchmark_group("catalog_representation/tag_capability_indexed_read");
    bench_tag_capability_dash(&mut group, size, fixture);
    bench_tag_capability_standard(&mut group, size, fixture);
    group.finish();
}

fn bench_tag_capability_dash(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    size: usize,
    fixture: &Fixture,
) {
    group.bench_with_input(
        BenchmarkId::new("dashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| {
                black_box(indexed_dash(
                    &fixture.dash,
                    &fixture.tag,
                    &fixture.capability,
                ))
            })
        },
    );
}

fn bench_tag_capability_standard(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    size: usize,
    fixture: &Fixture,
) {
    group.bench_with_input(
        BenchmarkId::new("standard_hashmap", size),
        fixture,
        |bench, fixture| {
            bench.iter(|| {
                black_box(indexed_standard(
                    &fixture.standard,
                    &fixture.tag,
                    &fixture.capability,
                ))
            })
        },
    );
}
