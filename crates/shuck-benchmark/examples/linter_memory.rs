use std::cell::RefCell;
use std::process;

use serde::Serialize;
use shuck_benchmark::{benchmark_cases, parse_fixture};
use shuck_indexer::Indexer;
use shuck_linter::{Checker, LinterFacts, LinterSemanticArtifacts, LinterSettings, ShellDialect};
use shuck_parser::parser::{ParseResult, ParseStatus};

#[global_allocator]
static GLOBAL: CountingAllocator<std::alloc::System> = CountingAllocator(std::alloc::System);

const MAX_MEASURE_DEPTH: usize = 8;

#[derive(Clone, Copy, Debug, Default)]
struct Frame {
    allocation_count: u64,
    reallocation_count: u64,
    total_allocated_bytes: u64,
    total_reallocated_bytes: u64,
    current_live_bytes: i64,
    peak_live_bytes: u64,
}

impl Frame {
    fn on_alloc(&mut self, size: usize) {
        self.allocation_count += 1;
        self.total_allocated_bytes += size as u64;
        self.current_live_bytes += size as i64;
        self.peak_live_bytes = self
            .peak_live_bytes
            .max(self.current_live_bytes.max(0) as u64);
    }

    fn on_dealloc(&mut self, size: usize) {
        self.current_live_bytes -= size as i64;
    }

    fn on_realloc(&mut self, old_size: usize, new_size: usize) {
        self.reallocation_count += 1;
        self.total_reallocated_bytes += new_size as u64;
        self.current_live_bytes += new_size as i64 - old_size as i64;
        self.peak_live_bytes = self
            .peak_live_bytes
            .max(self.current_live_bytes.max(0) as u64);
    }

    fn merge_sequential(&mut self, other: Frame) {
        self.allocation_count += other.allocation_count;
        self.reallocation_count += other.reallocation_count;
        self.total_allocated_bytes += other.total_allocated_bytes;
        self.total_reallocated_bytes += other.total_reallocated_bytes;
        let carried_live_bytes = self.current_live_bytes.max(0) as u64;
        self.peak_live_bytes = self
            .peak_live_bytes
            .max(carried_live_bytes + other.peak_live_bytes);
        self.current_live_bytes += other.current_live_bytes;
    }
}

#[derive(Debug, Default)]
struct CounterState {
    depth: usize,
    frames: [Frame; MAX_MEASURE_DEPTH],
}

thread_local! {
    static COUNTER_STATE: RefCell<CounterState> = RefCell::new(CounterState::default());
}

struct CountingAllocator<A>(A);

unsafe impl<A: std::alloc::GlobalAlloc> std::alloc::GlobalAlloc for CountingAllocator<A> {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let ptr = unsafe { self.0.alloc(layout) };
        if !ptr.is_null() {
            record_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        record_dealloc(layout.size());
        unsafe { self.0.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { self.0.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            record_realloc(layout.size(), new_size);
        }
        new_ptr
    }
}

fn record_alloc(size: usize) {
    COUNTER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let depth = state.depth;
        if depth == 0 {
            return;
        }
        for frame in &mut state.frames[1..=depth] {
            frame.on_alloc(size);
        }
    });
}

fn record_dealloc(size: usize) {
    COUNTER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let depth = state.depth;
        if depth == 0 {
            return;
        }
        for frame in &mut state.frames[1..=depth] {
            frame.on_dealloc(size);
        }
    });
}

fn record_realloc(old_size: usize, new_size: usize) {
    COUNTER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let depth = state.depth;
        if depth == 0 {
            return;
        }
        for frame in &mut state.frames[1..=depth] {
            frame.on_realloc(old_size, new_size);
        }
    });
}

fn measure<T>(f: impl FnOnce() -> T) -> (Frame, T) {
    COUNTER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        assert!(
            state.depth + 1 < MAX_MEASURE_DEPTH,
            "measurement nesting too deep"
        );
        state.depth += 1;
        let depth = state.depth;
        state.frames[depth] = Frame::default();
    });

    let result = f();

    let frame = COUNTER_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let depth = state.depth;
        let frame = state.frames[depth];
        state.frames[depth] = Frame::default();
        state.depth -= 1;
        frame
    });

    (frame, result)
}

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

struct PreparedInput {
    source: &'static str,
    output: ParseResult,
    indexer: Indexer,
    shell: ShellDialect,
}

impl PreparedInput {
    fn new(source: &'static str) -> Self {
        let output = parse_fixture(source);
        let indexer = Indexer::new(source, &output);
        let shell = ShellDialect::infer(source, None);

        Self {
            source,
            output,
            indexer,
            shell,
        }
    }
}

#[derive(Debug, Serialize)]
struct CaseReport {
    case: String,
    files: usize,
    recovered_files: usize,
    command_count: usize,
    fact_count: usize,
    diagnostic_count: usize,
    facts_metrics: MemoryMetrics,
    check_metrics: MemoryMetrics,
}

fn measured_facts_size(input: &PreparedInput) -> (Frame, usize) {
    let semantic = LinterSemanticArtifacts::build(&input.output.file, input.source, &input.indexer);
    measure(|| {
        let facts = LinterFacts::build_with_shell_and_ambient_shell_options(
            &input.output.file,
            input.source,
            &semantic,
            &input.indexer,
            input.shell,
            Default::default(),
        );

        facts.commands().len()
            + facts.word_facts().count()
            + facts.single_quoted_fragments().len()
            + facts.backtick_fragments().len()
            + facts.pattern_charclass_spans().len()
            + facts.substring_expansion_fragments().len()
            + facts.case_modification_fragments().len()
            + facts.replacement_expansion_fragments().len()
    })
}

fn measured_check_diagnostics(input: &PreparedInput, settings: &LinterSettings) -> (Frame, usize) {
    let semantic = LinterSemanticArtifacts::build(&input.output.file, input.source, &input.indexer);
    measure(|| {
        let checker = Checker::new(
            &input.output.file,
            input.source,
            &semantic,
            &input.indexer,
            &settings.rules,
            input.shell,
            settings.ambient_shell_options,
            settings.report_environment_style_names,
            settings.rule_options.clone(),
            None,
            None,
        );
        checker.check().len()
    })
}

fn sum_measured(values: impl IntoIterator<Item = (Frame, usize)>) -> (Frame, usize) {
    let mut total_frame = Frame::default();
    let mut total_count = 0;
    for (frame, count) in values {
        total_frame.merge_sequential(frame);
        total_count += count;
    }
    (total_frame, total_count)
}

fn single_case_report(case_name: &str) -> Option<CaseReport> {
    let cases = benchmark_cases();
    let case = cases.into_iter().find(|case| case.name == case_name)?;
    let inputs = case
        .files
        .iter()
        .map(|file| PreparedInput::new(file.source))
        .collect::<Vec<_>>();
    let settings = LinterSettings::default();

    let recovered_files = inputs
        .iter()
        .filter(|input| input.output.status != ParseStatus::Clean)
        .count();
    let command_count = inputs
        .iter()
        .map(|input| input.output.file.body.len())
        .sum();

    let (facts_frame, fact_count) = sum_measured(inputs.iter().map(measured_facts_size));
    let (check_frame, diagnostic_count) = sum_measured(
        inputs
            .iter()
            .map(|input| measured_check_diagnostics(input, &settings)),
    );

    Some(CaseReport {
        case: case.name.to_string(),
        files: case.files.len(),
        recovered_files,
        command_count,
        fact_count,
        diagnostic_count,
        facts_metrics: facts_frame.into(),
        check_metrics: check_frame.into(),
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
