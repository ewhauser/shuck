use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator, parse_fixture};
use shuck_indexer::Indexer;
use shuck_semantic::SemanticModel;

configure_benchmark_allocator!();

fn build_semantic(source: &str) -> usize {
    let output = parse_fixture(source);
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.script, source, &indexer);

    black_box(semantic.bindings().len() + semantic.references().len() + semantic.scopes().len())
}

fn bench_semantic(c: &mut Criterion) {
    let mut group = c.benchmark_group("semantic");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter(|| {
                let semantic_size: usize = case
                    .files
                    .iter()
                    .map(|file| build_semantic(file.source))
                    .sum();
                black_box(semantic_size);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_semantic);
criterion_main!(benches);
