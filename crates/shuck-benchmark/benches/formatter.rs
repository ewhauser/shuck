use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator, parse_fixture};
use shuck_formatter::{
    FormattedSource, ShellFormatOptions, build_comment_index, format_script_ast, format_source,
};

configure_benchmark_allocator!();

fn format_source_bytes(source: &str, options: &ShellFormatOptions) -> usize {
    let formatted = format_source(black_box(source), None, options)
        .expect("formatter benchmark inputs should format");

    output_bytes(source, formatted)
}

fn format_script_ast_bytes(
    source: &str,
    parsed: &shuck_parser::parser::ParseOutput,
    options: &ShellFormatOptions,
) -> usize {
    let formatted = format_script_ast(
        black_box(source),
        black_box(&parsed.script),
        black_box(&parsed.comments),
        None,
        options,
    )
    .expect("formatter AST benchmark inputs should format");

    output_bytes(source, formatted)
}

fn comment_index_items(source: &str, parsed: &shuck_parser::parser::ParseOutput) -> usize {
    black_box(build_comment_index(
        black_box(source),
        black_box(&parsed.comments),
    ))
}

fn output_bytes(source: &str, formatted: FormattedSource) -> usize {
    match formatted {
        FormattedSource::Unchanged => black_box(source.len()),
        FormattedSource::Formatted(formatted) => black_box(formatted.len()),
    }
}

fn bench_formatter(c: &mut Criterion) {
    let options = ShellFormatOptions::default();

    let mut source_group = c.benchmark_group("formatter_source");
    for case in benchmark_cases() {
        source_group.sample_size(case.speed.sample_size());
        source_group.throughput(Throughput::Bytes(case.total_bytes()));
        source_group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter(|| {
                let output_bytes: usize = case
                    .files
                    .iter()
                    .map(|file| format_source_bytes(file.source, &options))
                    .sum();
                black_box(output_bytes);
            });
        });
    }
    source_group.finish();

    let mut ast_group = c.benchmark_group("formatter_ast");
    for case in benchmark_cases() {
        let parsed_files = case
            .files
            .iter()
            .map(|file| parse_fixture(file.source))
            .collect::<Vec<_>>();

        ast_group.sample_size(case.speed.sample_size());
        ast_group.throughput(Throughput::Bytes(case.total_bytes()));
        ast_group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter(|| {
                let output_bytes: usize = case
                    .files
                    .iter()
                    .zip(parsed_files.iter())
                    .map(|(file, parsed)| format_script_ast_bytes(file.source, parsed, &options))
                    .sum();
                black_box(output_bytes);
            });
        });
    }
    ast_group.finish();

    let mut comments_group = c.benchmark_group("formatter_comments");
    for case in benchmark_cases() {
        let parsed_files = case
            .files
            .iter()
            .map(|file| parse_fixture(file.source))
            .collect::<Vec<_>>();

        comments_group.sample_size(case.speed.sample_size());
        comments_group.throughput(Throughput::Bytes(case.total_bytes()));
        comments_group.bench_with_input(
            BenchmarkId::from_parameter(case.name),
            &case,
            |b, case| {
                b.iter(|| {
                    let comment_items: usize = case
                        .files
                        .iter()
                        .zip(parsed_files.iter())
                        .map(|(file, parsed)| comment_index_items(file.source, parsed))
                        .sum();
                    black_box(comment_items);
                });
            },
        );
    }
    comments_group.finish();
}

criterion_group!(benches, bench_formatter);
criterion_main!(benches);
