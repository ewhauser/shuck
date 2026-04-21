use std::cell::RefCell;
use std::process;

use serde::Serialize;
use shuck_benchmark::benchmark_cases;
use shuck_parser::parser::{ParseResult, ParseStatus, Parser};

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
struct ParseMemoryMetrics {
    allocation_count: u64,
    reallocation_count: u64,
    total_allocated_bytes: u64,
    total_reallocated_bytes: u64,
    peak_live_bytes: u64,
    final_live_bytes: u64,
}

impl From<Frame> for ParseMemoryMetrics {
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
    metrics: ParseMemoryMetrics,
}

fn parse_source(source: &str) -> ParseResult {
    Parser::new(source).parse()
}

fn single_case_report(case_name: &str) -> Option<CaseReport> {
    let cases = benchmark_cases();
    let case = cases.into_iter().find(|case| case.name == case_name)?;

    if case.files.len() == 1 {
        let file = case.files[0];
        let (frame, output) = measure(|| parse_source(file.source));
        let recovered_files = usize::from(output.status != ParseStatus::Clean);
        let command_count = output.file.body.len();
        return Some(CaseReport {
            case: case.name.to_string(),
            files: 1,
            recovered_files,
            command_count,
            metrics: frame.into(),
        });
    }

    let (frame, (recovered_files, command_count)) = measure(|| {
        let mut recovered_files = 0usize;
        let mut command_count = 0usize;

        for file in case.files {
            let output = parse_source(file.source);
            recovered_files += usize::from(output.status != ParseStatus::Clean);
            command_count += output.file.body.len();
        }

        (recovered_files, command_count)
    });

    Some(CaseReport {
        case: case.name.to_string(),
        files: case.files.len(),
        recovered_files,
        command_count,
        metrics: frame.into(),
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
                "usage: cargo run -p shuck-benchmark --example parser_memory -- [--case NAME]"
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
