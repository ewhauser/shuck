use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use std::time::Duration;

use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator, parse_fixture};
use shuck_indexer::Indexer;
use shuck_linter::{
    LinterFacts, LinterSemanticArtifacts, LinterSettings, RuleSet, ShellCheckCodeMap,
    benchmark_collect_word_facts, benchmark_normalize_commands, lint_file_with_directives,
    parse_directives,
};
use shuck_parser::parser::ParseResult;
use shuck_semantic::SemanticModel;

configure_benchmark_allocator!();

struct PreparedFactsInput {
    source: &'static str,
    output: ParseResult,
    semantic: SemanticModel,
}

fn prepare_facts_input(source: &'static str) -> PreparedFactsInput {
    let output = parse_fixture(source);
    let indexer = Indexer::new(source, &output);
    let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer).into_semantic();

    PreparedFactsInput {
        source,
        output,
        semantic,
    }
}

fn build_linter_facts(
    source: &str,
    output: &ParseResult,
    indexer: &Indexer,
    semantic: &LinterSemanticArtifacts<'_>,
) -> usize {
    let facts = LinterFacts::build(&output.file, source, semantic, indexer);

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
    let diagnostics =
        lint_file_with_directives(&output, source, &indexer, settings, &directives, None);

    black_box(diagnostics.len())
}

fn bench_linter_facts(c: &mut Criterion) {
    let mut group = c.benchmark_group("linter_facts");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let outputs = case
                        .files
                        .iter()
                        .map(|file| parse_fixture(file.source))
                        .collect::<Vec<_>>();
                    let indexers = case
                        .files
                        .iter()
                        .zip(outputs.iter())
                        .map(|(file, output)| Indexer::new(file.source, output))
                        .collect::<Vec<_>>();
                    let semantics = case
                        .files
                        .iter()
                        .zip(outputs.iter())
                        .zip(indexers.iter())
                        .map(|((file, output), indexer)| {
                            LinterSemanticArtifacts::build(&output.file, file.source, indexer)
                        })
                        .collect::<Vec<_>>();

                    let start = std::time::Instant::now();
                    let facts_size = case
                        .files
                        .iter()
                        .zip(outputs.iter())
                        .zip(indexers.iter())
                        .zip(semantics.iter())
                        .map(|(((file, output), indexer), semantic)| {
                            build_linter_facts(file.source, output, indexer, semantic)
                        })
                        .sum::<usize>();
                    black_box(facts_size);
                    total += start.elapsed();
                }
                total
            });
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
    let settings = LinterSettings {
        rules: RuleSet::all(),
        ..LinterSettings::default()
    };
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
