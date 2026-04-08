use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator, parse_fixture};
use shuck_indexer::Indexer;
use shuck_semantic::SemanticModel;

configure_benchmark_allocator!();

fn build_semantic(source: &str) -> SemanticModel {
    let output = parse_fixture(source);
    let indexer = Indexer::new(source, &output);
    SemanticModel::build(&output.file, source, &indexer)
}

fn bench_unused_assignment(c: &mut Criterion) {
    let mut group = c.benchmark_group("unused_assignment");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter_batched(
                || {
                    case.files
                        .iter()
                        .map(|file| build_semantic(file.source))
                        .collect::<Vec<_>>()
                },
                |mut models| {
                    let unused_count: usize = models
                        .iter_mut()
                        .map(|model| model.precompute_unused_assignments().len())
                        .sum();
                    black_box(unused_count);
                },
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

fn bench_uninitialized_reference(c: &mut Criterion) {
    let mut group = c.benchmark_group("uninitialized_reference");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter_batched(
                || {
                    case.files
                        .iter()
                        .map(|file| build_semantic(file.source))
                        .collect::<Vec<_>>()
                },
                |mut models| {
                    let reference_count: usize = models
                        .iter_mut()
                        .map(|model| model.precompute_uninitialized_references().len())
                        .sum();
                    black_box(reference_count);
                },
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

fn bench_dead_code(c: &mut Criterion) {
    let mut group = c.benchmark_group("dead_code");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter_batched(
                || {
                    case.files
                        .iter()
                        .map(|file| build_semantic(file.source))
                        .collect::<Vec<_>>()
                },
                |mut models| {
                    let dead_code_count: usize = models
                        .iter_mut()
                        .map(|model| model.precompute_dead_code().len())
                        .sum();
                    black_box(dead_code_count);
                },
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_unused_assignment,
    bench_uninitialized_reference,
    bench_dead_code
);
criterion_main!(benches);
