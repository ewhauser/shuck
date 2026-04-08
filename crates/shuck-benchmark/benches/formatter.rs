use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator, parse_fixture};
use shuck_formatter::{
    BenchmarkFormatted, FormattedSource, ShellFormatOptions, build_benchmark_document,
    build_comment_index, format_file_ast, format_source,
};

configure_benchmark_allocator!();

fn format_source_bytes(source: &str, options: &ShellFormatOptions) -> usize {
    let formatted = format_source(black_box(source), None, options)
        .expect("formatter benchmark inputs should format");

    output_bytes(source, formatted)
}

fn format_file_ast_bytes(
    source: &str,
    parsed: shuck_parser::parser::ParseOutput,
    options: &ShellFormatOptions,
) -> usize {
    let formatted = format_file_ast(black_box(source), black_box(parsed.file), None, options)
        .expect("formatter AST benchmark inputs should format");

    output_bytes(source, formatted)
}

fn comment_index_items(source: &str, parsed: &shuck_parser::parser::ParseOutput) -> usize {
    black_box(build_comment_index(
        black_box(source),
        black_box(&parsed.file),
    ))
}

fn build_document_elements(
    source: &str,
    parsed: shuck_parser::parser::ParseOutput,
    options: &ShellFormatOptions,
) -> usize {
    let built = build_benchmark_document(black_box(source), black_box(parsed.file), None, options)
        .expect("formatter benchmark inputs should build");

    black_box(built.document_elements())
}

fn build_document_for_print<'a>(
    source: &'a str,
    parsed: &shuck_parser::parser::ParseOutput,
    options: &ShellFormatOptions,
) -> BenchmarkFormatted<'a> {
    build_benchmark_document(black_box(source), black_box(parsed.file.clone()), None, options)
        .expect("formatter benchmark inputs should build")
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
            b.iter_batched(
                || parsed_files.clone(),
                |parsed_files| {
                    let output_bytes: usize = case
                        .files
                        .iter()
                        .zip(parsed_files)
                        .map(|(file, parsed)| format_file_ast_bytes(file.source, parsed, &options))
                        .sum();
                    black_box(output_bytes);
                },
                BatchSize::LargeInput,
            );
        });
    }
    ast_group.finish();

    let mut build_group = c.benchmark_group("formatter_build_ast");
    for case in benchmark_cases() {
        let parsed_files = case
            .files
            .iter()
            .map(|file| parse_fixture(file.source))
            .collect::<Vec<_>>();

        build_group.sample_size(case.speed.sample_size());
        build_group.throughput(Throughput::Bytes(case.total_bytes()));
        build_group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter_batched(
                || parsed_files.clone(),
                |parsed_files| {
                    let document_elements: usize = case
                        .files
                        .iter()
                        .zip(parsed_files)
                        .map(|(file, parsed)| build_document_elements(file.source, parsed, &options))
                        .sum();
                    black_box(document_elements);
                },
                BatchSize::LargeInput,
            );
        });
    }
    build_group.finish();

    let mut print_group = c.benchmark_group("formatter_print_ast");
    for case in benchmark_cases() {
        let parsed_files = case
            .files
            .iter()
            .map(|file| parse_fixture(file.source))
            .collect::<Vec<_>>();
        let built_documents = case
            .files
            .iter()
            .zip(parsed_files.iter())
            .map(|(file, parsed)| build_document_for_print(file.source, parsed, &options))
            .collect::<Vec<_>>();

        print_group.sample_size(case.speed.sample_size());
        print_group.throughput(Throughput::Bytes(case.total_bytes()));
        print_group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, _case| {
            b.iter(|| {
                let output_bytes: usize = built_documents
                    .iter()
                    .map(|built| {
                        built
                            .print_bytes()
                            .expect("formatter benchmark documents should print")
                    })
                    .sum();
                black_box(output_bytes);
            });
        });
    }
    print_group.finish();

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
