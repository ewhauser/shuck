#![cfg_attr(not(test), warn(clippy::unwrap_used))]

use std::io::Write;
use std::process::ExitCode;

use colored::Colorize;

use shuck::args::Args;
use shuck::shellcheck_compat;
use shuck::{ExitStatus, run};

/// Native stack for every thread that parses, analyzes, or formats scripts.
///
/// The parser bounds its own recursion, but the semantic model, linter, and
/// formatter walk the AST recursively, and deeply nested scripts can exceed the
/// 2 MiB default for spawned threads. Release builds abort on stack overflow,
/// which would take down a whole multi-file run, so give these threads ample
/// headroom. The memory is only reserved; pages are committed as they are used.
const ANALYSIS_THREAD_STACK_SIZE: usize = 64 * 1024 * 1024;

fn main() -> ExitCode {
    // Ignore the error: it only means a global pool already exists.
    let _ = rayon::ThreadPoolBuilder::new()
        .stack_size(ANALYSIS_THREAD_STACK_SIZE)
        .build_global();

    // Run on a thread with the same stack so work done outside the pool, such
    // as stdin input or single-file paths, gets the same headroom.
    let worker = std::thread::Builder::new()
        .name("shuck-main".to_owned())
        .stack_size(ANALYSIS_THREAD_STACK_SIZE)
        .spawn(run_main);
    match worker {
        Ok(handle) => handle
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
        Err(_) => run_main(),
    }
}

fn run_main() -> ExitCode {
    #[cfg(windows)]
    assert!(colored::control::set_virtual_terminal(true).is_ok());

    let argv = std::env::args_os().collect::<Vec<_>>();
    if shellcheck_compat::should_activate(&argv) {
        return shellcheck_compat::run(argv);
    }

    let args = Args::try_parse_from(argv).unwrap_or_else(|err| err.exit());
    match run(args) {
        Ok(ExitStatus::Code(code)) => exit_with_child_status(code),
        Ok(status) => status.into(),
        Err(err) => report_error(&err),
    }
}

fn report_error(err: &anyhow::Error) -> ExitCode {
    for cause in err.chain() {
        if let Some(ioerr) = cause.downcast_ref::<std::io::Error>()
            && ioerr.kind() == std::io::ErrorKind::BrokenPipe
        {
            return ExitCode::from(0);
        }
    }

    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{}: {err}", "shuck".red().bold());
    for cause in err.chain().skip(1) {
        let _ = writeln!(stderr, "  {} {cause}", "Cause:".bold());
    }

    ExitStatus::Error.into()
}

#[cfg(windows)]
fn exit_with_child_status(code: i32) -> ExitCode {
    std::process::exit(code);
}

#[cfg(not(windows))]
fn exit_with_child_status(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
