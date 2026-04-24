use std::path::Path;
use std::process;

use serde::Serialize;
use shuck_benchmark::memory::{CountingAllocator, Frame, measure};
use shuck_benchmark::{benchmark_cases, parse_fixture};
use shuck_formatter::{FormattedSource, ShellFormatOptions, format_file_ast, format_source};

#[global_allocator]
static GLOBAL: CountingAllocator<std::alloc::System> = CountingAllocator(std::alloc::System);

#[derive(Debug, Serialize)]
struct MemoryMetrics {
    allocation_count: u64,
    reallocation_count: u64,
    total_allocated_bytes: u64,
    total_reallocated_bytes: u64,
    peak_live_bytes: u64,
    final_live_bytes: u64,
}

impl From<Frame> for MemoryMetrics {
    fn from(frame: Frame) -> Self {
        Self {
            allocation_count: frame.allocation_count,
            reallocation_count: frame.reallocation_count,
            total_allocated_bytes: frame.total_allocated_bytes,
            total_reallocated_bytes: frame.total_reallocated_bytes,
            peak_live_bytes: frame.peak_live_bytes,
            final_live_bytes: frame.current_live_bytes.max(0) as u64,
        }
    }
}

#[derive(Debug, Serialize)]
struct FormatterMetrics {
    output_bytes: usize,
    changed_files: usize,
    metrics: MemoryMetrics,
}

#[derive(Debug, Serialize)]
struct CaseReport {
    case: String,
    files: usize,
    source: FormatterMetrics,
    ast_only: FormatterMetrics,
}

fn output_bytes(source: &str, formatted: FormattedSource) -> (usize, bool) {
    match formatted {
        FormattedSource::Unchanged => (source.len(), false),
        FormattedSource::Formatted(formatted) => (formatted.len(), true),
    }
}

fn format_source_case(
    files: &[shuck_benchmark::TestFile],
    options: &ShellFormatOptions,
) -> FormatterMetrics {
    let (frame, (output_bytes, changed_files)) = measure(|| {
        let mut total_output_bytes = 0usize;
        let mut changed_files = 0usize;

        for file in files {
            let formatted = match format_source(file.source, None::<&Path>, options) {
                Ok(formatted) => formatted,
                Err(err) => panic!("formatter benchmark input should format: {err}"),
            };
            let (bytes, changed) = output_bytes(file.source, formatted);
            total_output_bytes += bytes;
            changed_files += usize::from(changed);
        }

        (total_output_bytes, changed_files)
    });

    FormatterMetrics {
        output_bytes,
        changed_files,
        metrics: frame.into(),
    }
}

fn format_ast_case(
    files: &[shuck_benchmark::TestFile],
    options: &ShellFormatOptions,
) -> FormatterMetrics {
    let parsed_files = files
        .iter()
        .map(|file| parse_fixture(file.source))
        .collect::<Vec<_>>();

    let (frame, (output_bytes, changed_files)) = measure(|| {
        let mut total_output_bytes = 0usize;
        let mut changed_files = 0usize;

        for (file, parsed) in files.iter().zip(parsed_files) {
            let formatted = match format_file_ast(file.source, parsed.file, None::<&Path>, options)
            {
                Ok(formatted) => formatted,
                Err(err) => panic!("formatter AST benchmark input should format: {err}"),
            };
            let (bytes, changed) = output_bytes(file.source, formatted);
            total_output_bytes += bytes;
            changed_files += usize::from(changed);
        }

        (total_output_bytes, changed_files)
    });

    FormatterMetrics {
        output_bytes,
        changed_files,
        metrics: frame.into(),
    }
}

fn single_case_report(case_name: &str) -> Option<CaseReport> {
    let cases = benchmark_cases();
    let case = cases.into_iter().find(|case| case.name == case_name)?;
    let options = ShellFormatOptions::default();

    Some(CaseReport {
        case: case.name.to_string(),
        files: case.files.len(),
        source: format_source_case(case.files, &options),
        ast_only: format_ast_case(case.files, &options),
    })
}

fn parse_case_arg() -> Option<String> {
    let mut args = std::env::args().skip(1);
    let arg = args.next()?;

    match arg.as_str() {
        "--case" => {
            let value = args.next();
            if let Some(extra) = args.next() {
                eprintln!("unknown argument `{extra}`");
                process::exit(2);
            }
            value
        }
        "--help" | "-h" => {
            eprintln!(
                "usage: cargo run -p shuck-benchmark --example formatter_memory -- [--case NAME]"
            );
            process::exit(0);
        }
        _ => {
            eprintln!("unknown argument `{arg}`");
            process::exit(2);
        }
    }
}

fn main() -> serde_json::Result<()> {
    let requested_case = parse_case_arg();
    let reports = if let Some(case_name) = requested_case {
        let Some(report) = single_case_report(&case_name) else {
            eprintln!("unknown benchmark case `{case_name}`");
            process::exit(2);
        };
        vec![report]
    } else {
        benchmark_cases()
            .into_iter()
            .filter_map(|case| single_case_report(case.name))
            .collect::<Vec<_>>()
    };

    serde_json::to_writer_pretty(std::io::stdout().lock(), &reports)?;
    println!();
    Ok(())
}
