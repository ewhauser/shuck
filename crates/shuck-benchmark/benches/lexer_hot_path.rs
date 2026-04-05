use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator};
use shuck_parser::parser::Lexer;

configure_benchmark_allocator!();

fn lex_source_hot_path(source: &str) -> usize {
    let mut lexer = Lexer::new(source);
    let mut token_count = 0usize;

    while let Some(_kind) = lexer.next_token_kind() {
        token_count += 1;
    }

    black_box(token_count)
}

fn bench_lexer_hot_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("lexer-hot-path");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter(|| {
                let token_count: usize = case
                    .files
                    .iter()
                    .map(|file| lex_source_hot_path(file.source))
                    .sum();
                black_box(token_count);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_lexer_hot_path);
criterion_main!(benches);
