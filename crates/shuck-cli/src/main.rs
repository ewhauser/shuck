#![cfg_attr(not(test), warn(clippy::unwrap_used))]

use std::io::Write;
use std::process::ExitCode;

use colored::Colorize;

use shuck::args::Args;
use shuck::shellcheck_compat;
use shuck::{ExitStatus, run};

fn main() -> ExitCode {
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
