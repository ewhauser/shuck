use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator, parse_fixture};
use shuck_indexer::Indexer;
use shuck_semantic::{EditorDocumentSymbol, SemanticModel};

configure_benchmark_allocator!();

struct PreparedEditorInput {
    semantic: SemanticModel,
    binding_count: u64,
}

fn prepare_editor_input(source: &'static str) -> PreparedEditorInput {
    let output = parse_fixture(source);
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let binding_count = semantic.bindings().len() as u64;

    PreparedEditorInput {
        semantic,
        binding_count,
    }
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

criterion_group!(benches, bench_editor_document_symbols);
criterion_main!(benches);
