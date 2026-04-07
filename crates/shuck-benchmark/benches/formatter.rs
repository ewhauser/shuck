use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator};
use shuck_formatter::{FormattedSource, ShellFormatOptions, format_source};

configure_benchmark_allocator!();

fn format_source_bytes(source: &str, options: &ShellFormatOptions) -> usize {
    let formatted = format_source(black_box(source), None, options)
        .expect("formatter benchmark inputs should format");

    match formatted {
        FormattedSource::Unchanged => black_box(source.len()),
        FormattedSource::Formatted(formatted) => black_box(formatted.len()),
    }
}

fn bench_formatter(c: &mut Criterion) {
    let mut group = c.benchmark_group("formatter");
    let options = ShellFormatOptions::default();

    for case in benchmark_cases() {
        group.sample_size(case.speed.sample_size());
        group.throughput(Throughput::Bytes(case.total_bytes()));
        group.bench_with_input(BenchmarkId::from_parameter(case.name), &case, |b, case| {
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

    group.finish();
}

criterion_group!(benches, bench_formatter);
criterion_main!(benches);
