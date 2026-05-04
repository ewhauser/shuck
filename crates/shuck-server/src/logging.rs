use serde::Deserialize;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Mutex;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::FmtSubscriber;

pub(crate) fn init_logging(log_level: LogLevel, log_file: Option<&Path>) {
    let builder = FmtSubscriber::builder()
        .with_max_level(log_level.level_filter())
        .with_ansi(false);
    if let Some(log_file) = log_file {
        match OpenOptions::new().create(true).append(true).open(log_file) {
            Ok(file) => {
                let _ = tracing::subscriber::set_global_default(
                    builder.with_writer(Mutex::new(file)).finish(),
                );
            }
            Err(error) => {
                eprintln!(
                    "failed to open shuck LSP log file {}: {error}",
                    log_file.display()
                );
                let _ = tracing::subscriber::set_global_default(builder.finish());
            }
        }
    } else {
        let _ = tracing::subscriber::set_global_default(builder.finish());
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Default, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn level_filter(self) -> LevelFilter {
        match self {
            Self::Error => LevelFilter::ERROR,
            Self::Warn => LevelFilter::WARN,
            Self::Info => LevelFilter::INFO,
            Self::Debug => LevelFilter::DEBUG,
            Self::Trace => LevelFilter::TRACE,
        }
    }
}
