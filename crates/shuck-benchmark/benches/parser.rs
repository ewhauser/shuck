use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator, parse_fixture};

configure_benchmark_allocator!();

fn parse_source(source: &str) -> usize {
    let output = parse_fixture(source);
    black_box(output.script.commands.len())
}

fn bench_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("parser");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter(|| {
                let command_count: usize = case
                    .files
                    .iter()
                    .map(|file| parse_source(file.source))
                    .sum();
                black_box(command_count);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_parser);
criterion_main!(benches);
