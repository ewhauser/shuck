use criterion::{
    BatchSize, BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main,
};
use shuck_benchmark::{TestFile, benchmark_cases, configure_benchmark_allocator, parse_fixture};
use shuck_indexer::Indexer;
use shuck_semantic::{EditorCompletionOptions, EditorDocumentSymbol, SemanticModel};

configure_benchmark_allocator!();

struct PreparedEditorInput {
    source: &'static str,
    indexer: Indexer,
    semantic: SemanticModel,
    binding_count: u64,
    hover_probes: Vec<usize>,
    completion_probes: Vec<usize>,
}

fn prepare_editor_input(source: &'static str) -> PreparedEditorInput {
    let output = parse_fixture(source);
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let binding_count = semantic.bindings().len() as u64;
    let hover_probes = hover_probes(source, &semantic);
    let completion_probes = completion_probes(source);

    PreparedEditorInput {
        source,
        indexer,
        semantic,
        binding_count,
        hover_probes,
        completion_probes,
    }
}

fn prepare_editor_inputs(files: &'static [TestFile]) -> Vec<PreparedEditorInput> {
    files
        .iter()
        .map(|file| prepare_editor_input(file.source))
        .collect()
}

fn count_document_symbols(symbols: &[EditorDocumentSymbol]) -> usize {
    symbols
        .iter()
        .map(|symbol| 1 + count_document_symbols(&symbol.children))
        .sum()
}

fn build_document_symbols(input: &PreparedEditorInput) -> usize {
    // Keep parse/index/semantic construction out of the timed loop so this
    // benchmark isolates the editor query projection itself.
    let symbols = input.semantic.editor_query().document_symbols();
    black_box(count_document_symbols(&symbols))
}

fn hover_probes(source: &str, semantic: &SemanticModel) -> Vec<usize> {
    let mut probes = Vec::new();
    probes.extend(
        semantic
            .bindings()
            .iter()
            .filter(|binding| binding.span.start.offset < binding.span.end.offset)
            .take(64)
            .map(|binding| binding.span.start.offset),
    );
    probes.extend(
        semantic
            .references()
            .iter()
            .filter(|reference| reference.span.start.offset < reference.span.end.offset)
            .take(64)
            .map(|reference| reference.span.start.offset),
    );
    for binding in semantic.function_definition_bindings().take(32) {
        probes.extend(
            semantic
                .call_sites_for(&binding.name)
                .iter()
                .filter(|site| site.name_span.start.offset < site.name_span.end.offset)
                .take(2)
                .map(|site| site.name_span.start.offset),
        );
    }
    probes.extend(miss_probes(source).take(16));
    probes.sort_unstable();
    probes.dedup();
    if probes.is_empty() {
        probes.push(0);
    }
    probes
}

fn miss_probes(source: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(source.match_indices('\n').map(|(offset, _)| offset))
}

fn run_hover_queries(input: &PreparedEditorInput) -> usize {
    let query = input.semantic.editor_query();
    let hit_count = input
        .hover_probes
        .iter()
        .filter(|offset| query.hover_at_offset(**offset).is_some())
        .count();
    black_box(hit_count)
}

fn run_hover_queries_for_inputs(inputs: &[PreparedEditorInput]) -> usize {
    inputs.iter().map(run_hover_queries).sum()
}

fn completion_probes(source: &str) -> Vec<usize> {
    let mut probes = source
        .match_indices('$')
        .take(64)
        .map(|(offset, _)| offset + 1)
        .collect::<Vec<_>>();
    probes.extend(
        source
            .match_indices('\n')
            .take(64)
            .map(|(offset, _)| offset + 1),
    );
    probes.sort_unstable();
    probes.dedup();
    if probes.is_empty() {
        probes.push(source.len());
    }
    probes
}

fn run_completion_queries(input: &PreparedEditorInput) -> usize {
    let query = input.semantic.editor_query();
    let count = input
        .completion_probes
        .iter()
        .filter_map(|offset| {
            query.completions_at_offset(
                input.source,
                &input.indexer,
                *offset,
                EditorCompletionOptions::default(),
            )
        })
        .map(|completions| completions.items.len())
        .sum();
    black_box(count)
}

fn run_completion_queries_for_inputs(inputs: &[PreparedEditorInput]) -> usize {
    inputs.iter().map(run_completion_queries).sum()
}

fn run_definition_queries(input: &PreparedEditorInput) -> usize {
    let query = input.semantic.editor_query();
    let count = input
        .hover_probes
        .iter()
        .map(|offset| query.definition_spans_at_offset(*offset).len())
        .sum();
    black_box(count)
}

fn run_definition_queries_for_inputs(inputs: &[PreparedEditorInput]) -> usize {
    inputs.iter().map(run_definition_queries).sum()
}

fn run_references_queries(input: &PreparedEditorInput) -> usize {
    let query = input.semantic.editor_query();
    let count = input
        .hover_probes
        .iter()
        .map(|offset| query.occurrences_at_offset(*offset, true).len())
        .sum();
    black_box(count)
}

fn run_references_queries_for_inputs(inputs: &[PreparedEditorInput]) -> usize {
    inputs.iter().map(run_references_queries).sum()
}

fn run_document_highlight_queries(input: &PreparedEditorInput) -> usize {
    let query = input.semantic.editor_query();
    let count = input
        .hover_probes
        .iter()
        .map(|offset| query.occurrences_at_offset(*offset, true).len())
        .sum();
    black_box(count)
}

fn run_document_highlight_queries_for_inputs(inputs: &[PreparedEditorInput]) -> usize {
    inputs.iter().map(run_document_highlight_queries).sum()
}

fn run_rename_queries(input: &PreparedEditorInput) -> usize {
    let query = input.semantic.editor_query();
    let count = input
        .hover_probes
        .iter()
        .filter_map(|offset| query.rename_set_at_offset(*offset).ok())
        .map(|rename| rename.spans.len())
        .sum();
    black_box(count)
}

fn run_rename_queries_for_inputs(inputs: &[PreparedEditorInput]) -> usize {
    inputs.iter().map(run_rename_queries).sum()
}

fn hover_probe_count(inputs: &[PreparedEditorInput]) -> u64 {
    inputs
        .iter()
        .map(|input| input.hover_probes.len() as u64)
        .sum::<u64>()
        .max(1)
}

fn completion_probe_count(inputs: &[PreparedEditorInput]) -> u64 {
    inputs
        .iter()
        .map(|input| input.completion_probes.len() as u64)
        .sum::<u64>()
        .max(1)
}

fn bench_editor_document_symbols(c: &mut Criterion) {
    let mut group = c.benchmark_group("editor_document_symbols");

    for case in benchmark_cases() {
        let inputs = case
            .files
            .iter()
            .map(|file| prepare_editor_input(file.source))
            .collect::<Vec<_>>();
        let total_bindings = inputs
            .iter()
            .map(|input| input.binding_count)
            .sum::<u64>()
            .max(1);

        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Elements(total_bindings));
        group.bench_function(BenchmarkId::from_parameter(case.name), move |b| {
            b.iter(|| {
                let symbol_count: usize = inputs.iter().map(build_document_symbols).sum();
                black_box(symbol_count);
            });
        });
    }

    group.finish();
}

fn bench_editor_hover(c: &mut Criterion) {
    let mut cold_group = c.benchmark_group("editor_hover_cold");
    for case in benchmark_cases() {
        let probe_count = hover_probe_count(&prepare_editor_inputs(case.files));
        cold_group.sample_size(case.speed.sample_size());
        cold_group.throughput(Throughput::Elements(probe_count));
        cold_group.bench_function(BenchmarkId::from_parameter(case.name), move |b| {
            b.iter_batched(
                || prepare_editor_inputs(case.files),
                |inputs| run_hover_queries_for_inputs(&inputs),
                BatchSize::SmallInput,
            );
        });
    }
    cold_group.finish();

    let mut warm_group = c.benchmark_group("editor_hover_warm");
    for case in benchmark_cases() {
        let inputs = prepare_editor_inputs(case.files);
        let probe_count = hover_probe_count(&inputs);
        for input in &inputs {
            let _ = run_hover_queries(input);
        }

        warm_group.sample_size(case.speed.sample_size());
        warm_group.throughput(Throughput::Elements(probe_count));
        warm_group.bench_function(BenchmarkId::from_parameter(case.name), move |b| {
            b.iter(|| run_hover_queries_for_inputs(&inputs));
        });
    }
    warm_group.finish();
}

fn bench_editor_completion(c: &mut Criterion) {
    let mut group = c.benchmark_group("editor_completion");
    for case in benchmark_cases() {
        let inputs = prepare_editor_inputs(case.files);
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Elements(completion_probe_count(&inputs)));
        group.bench_function(BenchmarkId::from_parameter(case.name), move |b| {
            b.iter(|| run_completion_queries_for_inputs(&inputs));
        });
    }
    group.finish();
}

fn bench_editor_navigation(c: &mut Criterion) {
    let mut definition_group = c.benchmark_group("editor_definition");
    for case in benchmark_cases() {
        let inputs = prepare_editor_inputs(case.files);
        definition_group.sample_size(case.speed.sample_size());
        definition_group.throughput(Throughput::Elements(hover_probe_count(&inputs)));
        definition_group.bench_function(BenchmarkId::from_parameter(case.name), move |b| {
            b.iter(|| run_definition_queries_for_inputs(&inputs));
        });
    }
    definition_group.finish();

    let mut references_group = c.benchmark_group("editor_references");
    for case in benchmark_cases() {
        let inputs = prepare_editor_inputs(case.files);
        references_group.sample_size(case.speed.sample_size());
        references_group.throughput(Throughput::Elements(hover_probe_count(&inputs)));
        references_group.bench_function(BenchmarkId::from_parameter(case.name), move |b| {
            b.iter(|| run_references_queries_for_inputs(&inputs));
        });
    }
    references_group.finish();

    let mut highlight_group = c.benchmark_group("editor_document_highlight");
    for case in benchmark_cases() {
        let inputs = prepare_editor_inputs(case.files);
        highlight_group.sample_size(case.speed.sample_size());
        highlight_group.throughput(Throughput::Elements(hover_probe_count(&inputs)));
        highlight_group.bench_function(BenchmarkId::from_parameter(case.name), move |b| {
            b.iter(|| run_document_highlight_queries_for_inputs(&inputs));
        });
    }
    highlight_group.finish();
}

fn bench_editor_rename(c: &mut Criterion) {
    let mut group = c.benchmark_group("editor_rename");
    for case in benchmark_cases() {
        let inputs = prepare_editor_inputs(case.files);
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Elements(hover_probe_count(&inputs)));
        group.bench_function(BenchmarkId::from_parameter(case.name), move |b| {
            b.iter(|| run_rename_queries_for_inputs(&inputs));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_editor_document_symbols,
    bench_editor_hover,
    bench_editor_completion,
    bench_editor_navigation,
    bench_editor_rename
);
criterion_main!(benches);
