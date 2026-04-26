use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator, parse_fixture};
use shuck_indexer::Indexer;
use shuck_parser::parser::ParseResult;

configure_benchmark_allocator!();

struct PreparedInput {
    source: &'static str,
    output: ParseResult,
}

fn prepare_input(source: &'static str) -> PreparedInput {
    let output = parse_fixture(source);
    PreparedInput { source, output }
}

fn build_indexer(input: &PreparedInput) -> usize {
    let indexer = Indexer::new(input.source, &input.output);
    black_box(
        indexer.line_index().line_count()
            + indexer.comment_index().comments().len()
            + indexer.continuation_line_starts().len(),
    )
}

fn bench_indexer(c: &mut Criterion) {
    let mut group = c.benchmark_group("indexer");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            let inputs = case
                .files
                .iter()
                .map(|file| prepare_input(file.source))
                .collect::<Vec<_>>();

            b.iter(|| {
                let indexed_size: usize = inputs.iter().map(build_indexer).sum();
                black_box(indexed_size);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_indexer);
criterion_main!(benches);
