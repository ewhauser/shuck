use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator, parse_fixture};
use shuck_indexer::Indexer;
use shuck_linter::{
    LinterSettings, ShellCheckCodeMap, SuppressionIndex, first_statement_line, lint_file,
    parse_directives,
};

configure_benchmark_allocator!();

fn lint_source(
    source: &str,
    settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
) -> usize {
    let output = parse_fixture(source);
    let indexer = Indexer::new(source, &output);
    let directives = parse_directives(source, indexer.comment_index(), shellcheck_map);
    let suppression_index = (!directives.is_empty()).then(|| {
        SuppressionIndex::new(
            &directives,
            &output.file,
            first_statement_line(&output.file).unwrap_or(u32::MAX),
        )
    });
    let diagnostics = lint_file(
        &output.file,
        source,
        &indexer,
        settings,
        suppression_index.as_ref(),
    );

    black_box(diagnostics.len())
}

fn bench_linter(c: &mut Criterion) {
    let mut group = c.benchmark_group("linter");
    let settings = LinterSettings::default();
    let shellcheck_map = ShellCheckCodeMap::default();

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter(|| {
                let diagnostic_count: usize = case
                    .files
                    .iter()
                    .map(|file| lint_source(file.source, &settings, &shellcheck_map))
                    .sum();
                black_box(diagnostic_count);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_linter);
criterion_main!(benches);
