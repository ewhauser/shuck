use std::io::Write;
use std::process::ExitCode;

use colored::Colorize;

use shuck::args::Args;
use shuck::{ExitStatus, run};

fn main() -> ExitCode {
    #[cfg(windows)]
    assert!(colored::control::set_virtual_terminal(true).is_ok());

    let args = Args::parse();
    match run(args) {
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
