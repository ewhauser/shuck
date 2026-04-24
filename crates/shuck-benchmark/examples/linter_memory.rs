use std::process;

use serde::Serialize;
use shuck_benchmark::benchmark_cases;
use shuck_benchmark::memory::{CountingAllocator, Frame, measure};
use shuck_indexer::Indexer;
use shuck_linter::{
    LinterSettings, ShellCheckCodeMap, SuppressionIndex, first_statement_line, lint_file,
    parse_directives,
};
use shuck_parser::parser::{ParseStatus, Parser};

#[global_allocator]
static GLOBAL: CountingAllocator<std::alloc::System> = CountingAllocator(std::alloc::System);

#[derive(Debug, Default, Serialize)]
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
struct CaseReport {
    case: String,
    files: usize,
    recovered_files: usize,
    command_count: usize,
    diagnostic_count: usize,
    full: MemoryMetrics,
    phases: PhaseMetrics,
}

#[derive(Debug, Default, Serialize)]
struct PhaseMetrics {
    parse: MemoryMetrics,
    index_and_suppressions: MemoryMetrics,
    lint: MemoryMetrics,
}

impl PhaseMetrics {
    fn add_parse(&mut self, frame: Frame) {
        add_metrics(&mut self.parse, frame);
    }

    fn add_index_and_suppressions(&mut self, frame: Frame) {
        add_metrics(&mut self.index_and_suppressions, frame);
    }

    fn add_lint(&mut self, frame: Frame) {
        add_metrics(&mut self.lint, frame);
    }
}

fn add_metrics(metrics: &mut MemoryMetrics, frame: Frame) {
    metrics.allocation_count += frame.allocation_count;
    metrics.reallocation_count += frame.reallocation_count;
    metrics.total_allocated_bytes += frame.total_allocated_bytes;
    metrics.total_reallocated_bytes += frame.total_reallocated_bytes;
    metrics.peak_live_bytes = metrics.peak_live_bytes.max(frame.peak_live_bytes);
    metrics.final_live_bytes += frame.current_live_bytes.max(0) as u64;
}

struct FileLintReport {
    command_count: usize,
    diagnostic_count: usize,
    recovered: bool,
    parse_frame: Frame,
    index_and_suppressions_frame: Frame,
    lint_frame: Frame,
}

fn lint_source_with_phases(
    source: &str,
    settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
) -> FileLintReport {
    let (parse_frame, output) = measure(|| Parser::new(source).parse());
    let (index_and_suppressions_frame, (indexer, suppression_index)) = measure(|| {
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

        (indexer, suppression_index)
    });
    let (lint_frame, diagnostics) = measure(|| {
        lint_file(
            &output,
            source,
            &indexer,
            settings,
            suppression_index.as_ref(),
            None,
        )
    });

    FileLintReport {
        command_count: output.file.body.len(),
        diagnostic_count: diagnostics.len(),
        recovered: output.status != ParseStatus::Clean,
        parse_frame,
        index_and_suppressions_frame,
        lint_frame,
    }
}

fn single_case_report(case_name: &str) -> Option<CaseReport> {
    let cases = benchmark_cases();
    let case = cases.into_iter().find(|case| case.name == case_name)?;
    let settings = LinterSettings::default();
    let shellcheck_map = ShellCheckCodeMap::default();

    let (frame, (recovered_files, command_count, diagnostic_count, phases)) = measure(|| {
        let mut recovered_files = 0usize;
        let mut command_count = 0usize;
        let mut diagnostic_count = 0usize;
        let mut phases = PhaseMetrics::default();

        for file in case.files {
            let report = lint_source_with_phases(file.source, &settings, &shellcheck_map);
            command_count += report.command_count;
            diagnostic_count += report.diagnostic_count;
            recovered_files += usize::from(report.recovered);
            phases.add_parse(report.parse_frame);
            phases.add_index_and_suppressions(report.index_and_suppressions_frame);
            phases.add_lint(report.lint_frame);
        }

        (recovered_files, command_count, diagnostic_count, phases)
    });

    Some(CaseReport {
        case: case.name.to_string(),
        files: case.files.len(),
        recovered_files,
        command_count,
        diagnostic_count,
        full: frame.into(),
        phases,
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
                "usage: cargo run -p shuck-benchmark --example linter_memory -- [--case NAME]"
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
