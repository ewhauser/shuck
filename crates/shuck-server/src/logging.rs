use serde::Deserialize;
use std::path::Path;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::FmtSubscriber;

pub(crate) fn init_logging(log_level: LogLevel, _log_file: Option<&Path>) {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(log_level.level_filter())
        .with_ansi(false)
        .finish();

    let _ = tracing::subscriber::set_global_default(subscriber);
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
