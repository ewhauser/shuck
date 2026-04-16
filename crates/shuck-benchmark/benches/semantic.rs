use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator, parse_fixture};
use shuck_indexer::Indexer;
use shuck_semantic::{ScopeKind, SemanticModel};
use std::collections::HashSet;

configure_benchmark_allocator!();

struct PreparedScopeLookupInput {
    semantic: SemanticModel,
    offsets: Vec<usize>,
}

fn build_semantic(source: &str) -> usize {
    let output = parse_fixture(source);
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);

    black_box(semantic.bindings().len() + semantic.references().len() + semantic.scopes().len())
}

fn prepare_scope_lookup_input(source: &'static str) -> PreparedScopeLookupInput {
    let output = parse_fixture(source);
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let mut seen = HashSet::new();
    let mut offsets = Vec::new();

    for offset in semantic
        .scopes()
        .iter()
        .map(|scope| scope.span.start.offset)
        .chain(
            semantic
                .bindings()
                .iter()
                .map(|binding| binding.span.start.offset),
        )
        .chain(
            semantic
                .references()
                .iter()
                .map(|reference| reference.span.start.offset),
        )
    {
        if seen.insert(offset) {
            offsets.push(offset);
        }
    }

    if offsets.is_empty() {
        offsets.push(0);
    }

    PreparedScopeLookupInput { semantic, offsets }
}

fn run_scope_lookups(input: &PreparedScopeLookupInput) -> usize {
    let mut score = 0usize;

    for &offset in &input.offsets {
        let scope = input.semantic.scope_at(black_box(offset));
        score += match input.semantic.scope_kind(scope) {
            ScopeKind::File => 1,
            ScopeKind::Function(_) => 3,
            ScopeKind::Subshell => 5,
            ScopeKind::CommandSubstitution => 7,
            ScopeKind::Pipeline => 11,
        };
    }

    black_box(score)
}

fn bench_semantic(c: &mut Criterion) {
    let mut group = c.benchmark_group("semantic");

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
            b.iter(|| {
                let semantic_size: usize = case
                    .files
                    .iter()
                    .map(|file| build_semantic(file.source))
                    .sum();
                black_box(semantic_size);
            });
        });
    }

    group.finish();
}

fn bench_semantic_scope_at(c: &mut Criterion) {
    let mut group = c.benchmark_group("semantic_scope_at");

    for case in benchmark_cases() {
        let inputs = case
            .files
            .iter()
            .map(|file| prepare_scope_lookup_input(file.source))
            .collect::<Vec<_>>();
        let total_lookups = inputs.iter().map(|input| input.offsets.len() as u64).sum();

        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Elements(total_lookups));
        group.bench_function(BenchmarkId::from_parameter(case.name), move |b| {
            b.iter(|| {
                let score: usize = inputs.iter().map(run_scope_lookups).sum();
                black_box(score);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_semantic, bench_semantic_scope_at);
criterion_main!(benches);
