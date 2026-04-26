use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator, parse_fixture};
use shuck_indexer::Indexer;
use shuck_linter::{
    LinterFacts, LinterSettings, ShellCheckCodeMap, ShellDialect, SuppressionIndex,
    benchmark_collect_word_facts, benchmark_normalize_commands, classify_file_context,
    first_statement_line, lint_file, parse_directives,
};
use shuck_parser::parser::ParseResult;
use shuck_semantic::SemanticModel;

configure_benchmark_allocator!();

struct PreparedFactsInput {
    source: &'static str,
    output: ParseResult,
    indexer: Indexer,
    semantic: SemanticModel,
    file_context: shuck_linter::FileContext,
}

fn prepare_facts_input(source: &'static str) -> PreparedFactsInput {
    let output = parse_fixture(source);
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build_arena(&output.arena_file, source, &indexer);
    let shell = ShellDialect::infer(source, None);
    let file_context = classify_file_context(source, None, shell);

    PreparedFactsInput {
        source,
        output,
        indexer,
        semantic,
        file_context,
    }
}

fn build_linter_facts(input: &PreparedFactsInput) -> usize {
    let facts = LinterFacts::build(
        &input.output.file,
        input.source,
        &input.semantic,
        &input.indexer,
        &input.file_context,
    );

    black_box(
        facts.commands().len()
            + facts.word_facts().count()
            + facts.single_quoted_fragments().len()
            + facts.backtick_fragments().len()
            + facts.pattern_charclass_spans().len()
            + facts.substring_expansion_fragments().len()
            + facts.case_modification_fragments().len()
            + facts.replacement_expansion_fragments().len(),
    )
}

fn build_normalized_commands(input: &PreparedFactsInput) -> usize {
    black_box(benchmark_normalize_commands(
        &input.output.file,
        input.source,
    ))
}

fn build_word_facts(input: &PreparedFactsInput) -> usize {
    black_box(benchmark_collect_word_facts(
        &input.output.file,
        input.source,
        &input.semantic,
    ))
}

fn lint_source(
    source: &str,
    settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
) -> usize {
    let output = parse_fixture(source);
    let indexer = Indexer::new(source, &output);
    let directives = parse_directives(
        source,
        &output.file,
        indexer.comment_index(),
        shellcheck_map,
    );
    let suppression_index = (!directives.is_empty()).then(|| {
        SuppressionIndex::new(
            &directives,
            &output.file,
            first_statement_line(&output.file).unwrap_or(u32::MAX),
        )
    });
    let diagnostics = lint_file(
        &output,
        source,
        &indexer,
        settings,
        suppression_index.as_ref(),
        None,
    );

    black_box(diagnostics.len())
}

fn bench_linter_facts(c: &mut Criterion) {
    let mut group = c.benchmark_group("linter_facts");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter_batched(
                || {
                    case.files
                        .iter()
                        .map(|file| prepare_facts_input(file.source))
                        .collect::<Vec<_>>()
                },
                |inputs| {
                    let facts_size: usize = inputs.iter().map(build_linter_facts).sum();
                    black_box(facts_size);
                },
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

fn bench_linter_normalization(c: &mut Criterion) {
    let mut group = c.benchmark_group("linter_normalization");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter_batched(
                || {
                    case.files
                        .iter()
                        .map(|file| prepare_facts_input(file.source))
                        .collect::<Vec<_>>()
                },
                |inputs| {
                    let normalized_size: usize = inputs.iter().map(build_normalized_commands).sum();
                    black_box(normalized_size);
                },
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

fn bench_linter_word_facts(c: &mut Criterion) {
    let mut group = c.benchmark_group("linter_word_facts");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter_batched(
                || {
                    case.files
                        .iter()
                        .map(|file| prepare_facts_input(file.source))
                        .collect::<Vec<_>>()
                },
                |inputs| {
                    let facts_size: usize = inputs.iter().map(build_word_facts).sum();
                    black_box(facts_size);
                },
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
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

criterion_group!(
    benches,
    bench_linter_facts,
    bench_linter_normalization,
    bench_linter_word_facts,
    bench_linter
);
criterion_main!(benches);
