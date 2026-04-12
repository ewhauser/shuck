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
                |models| {
                    let unused_count: usize = models
                        .iter()
                        .map(|model| model.analysis().unused_assignments().len())
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
                |models| {
                    let reference_count: usize = models
                        .iter()
                        .map(|model| model.analysis().uninitialized_references().len())
                        .sum();
                    black_box(reference_count);
                },
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

fn bench_variable_dataflow_combined(c: &mut Criterion) {
    let mut group = c.benchmark_group("variable_dataflow_combined");

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
                |models| {
                    let combined_count: usize = models
                        .iter()
                        .map(|model| {
                            let analysis = model.analysis();
                            analysis.unused_assignments().len()
                                + analysis.uninitialized_references().len()
                        })
                        .sum();
                    black_box(combined_count);
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
                |models| {
                    let dead_code_count: usize = models
                        .iter()
                        .map(|model| model.analysis().dead_code().len())
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
    bench_variable_dataflow_combined,
    bench_dead_code
);
criterion_main!(benches);
