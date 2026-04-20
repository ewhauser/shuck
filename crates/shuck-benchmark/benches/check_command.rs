use std::fs;
use std::path::PathBuf;

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use shuck::args::CheckOutputFormatArg;
use shuck_benchmark::{benchmark_cases, configure_benchmark_allocator};
use tempfile::TempDir;

configure_benchmark_allocator!();

struct PreparedCheckCase {
    _tempdir: TempDir,
    cwd: PathBuf,
    paths: Vec<PathBuf>,
}

fn prepare_check_case(case: shuck_benchmark::TestCase) -> PreparedCheckCase {
    let tempdir = tempfile::tempdir().expect("benchmark tempdir should exist");
    let cwd = tempdir.path().to_path_buf();
    let mut paths = Vec::with_capacity(case.files.len());

    for (index, file) in case.files.iter().enumerate() {
        let path = cwd.join(format!("{index:02}-{}.sh", file.name));
        fs::write(&path, file.source).expect("benchmark fixture should write");
        paths.push(path);
    }

    PreparedCheckCase {
        _tempdir: tempdir,
        cwd,
        paths,
    }
}

fn bench_check_command(c: &mut Criterion) {
    let cases = benchmark_cases();

    for output_format in [CheckOutputFormatArg::Concise, CheckOutputFormatArg::Full] {
        let group_name = match output_format {
            CheckOutputFormatArg::Concise => "check_command_concise",
            CheckOutputFormatArg::Full => "check_command_full",
        };
        let mut group = c.benchmark_group(group_name);

        for case in &cases {
            group.sample_size(case.speed.sample_size());
            group.throughput(Throughput::Bytes(case.total_bytes()));
            let prepared = prepare_check_case(*case);

            group.bench_with_input(
                BenchmarkId::from_parameter(case.name),
                &prepared,
                |b, input| {
                    b.iter(|| {
                        black_box(
                            shuck::benchmark_check_paths(&input.cwd, &input.paths, output_format)
                                .expect("check benchmark should succeed"),
                        )
                    });
                },
            );
        }

        group.finish();
    }
}

criterion_group!(benches, bench_check_command);
criterion_main!(benches);
