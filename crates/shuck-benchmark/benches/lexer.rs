use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator};
use shuck_parser::parser::Lexer;

configure_benchmark_allocator!();

fn lex_source(source: &str) -> usize {
    let mut lexer = Lexer::new(source);
    let mut tokens = Vec::new();

    while let Some(token) = lexer.next_token() {
        tokens.push(token);
    }

    black_box(tokens.len())
}

fn bench_lexer(c: &mut Criterion) {
    let mut group = c.benchmark_group("lexer");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter(|| {
                let token_count: usize =
                    case.files.iter().map(|file| lex_source(file.source)).sum();
                black_box(token_count);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_lexer);
criterion_main!(benches);
