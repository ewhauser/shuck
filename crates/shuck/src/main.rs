use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

use shuck::args::Args;
use shuck::{ExitStatus, run};

fn main() -> ExitCode {
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
    let _ = writeln!(stderr, "shuck: {err}");
    for cause in err.chain().skip(1) {
        let _ = writeln!(stderr, "  caused by: {cause}");
    }

    ExitStatus::Error.into()
}
