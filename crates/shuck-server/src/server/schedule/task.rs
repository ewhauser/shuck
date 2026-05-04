use lsp_server::RequestId;
use serde::Serialize;

use crate::session::{Client, Session};

type LocalFn = Box<dyn FnOnce(&mut Session, &Client)>;
type BackgroundFn = Box<dyn FnOnce(&Client) + Send + 'static>;
type BackgroundFnBuilder = Box<dyn FnOnce(&Session) -> BackgroundFn>;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::server) enum BackgroundSchedule {
    Fmt,
    #[default]
    Worker,
    LatencySensitive,
}

#[must_use]
pub(in crate::server) enum Task {
    Background(BackgroundTaskBuilder),
    Sync(SyncTask),
}

pub(in crate::server) struct BackgroundTaskBuilder {
    pub(super) schedule: BackgroundSchedule,
    pub(super) builder: BackgroundFnBuilder,
}

pub(in crate::server) struct SyncTask {
    pub(super) func: LocalFn,
}

impl Task {
    pub(crate) fn background(
        schedule: BackgroundSchedule,
        func: impl FnOnce(&Session) -> Box<dyn FnOnce(&Client) + Send + 'static> + 'static,
    ) -> Self {
        Self::Background(BackgroundTaskBuilder {
            schedule,
            builder: Box::new(func),
        })
    }

    pub(crate) fn sync(func: impl FnOnce(&mut Session, &Client) + 'static) -> Self {
        Self::Sync(SyncTask {
            func: Box::new(func),
        })
    }

    pub(crate) fn immediate<R>(id: RequestId, result: crate::server::Result<R>) -> Self
    where
        R: Serialize + Send + 'static,
    {
        Self::sync(move |_, client| {
            if let Err(err) = client.respond(&id, result) {
                tracing::error!("Unable to send immediate response: {err}");
            }
        })
    }

    pub(crate) fn nothing() -> Self {
        Self::sync(|_, _| {})
    }

    #[cfg(test)]
    pub(crate) fn run_for_test(self, session: &mut Session, client: &Client) {
        match self {
            Self::Background(BackgroundTaskBuilder { builder, .. }) => builder(session)(client),
            Self::Sync(SyncTask { func }) => func(session, client),
        }
    }

    #[cfg(test)]
    pub(crate) fn build_background_for_test(self, session: &Session) -> Option<BackgroundFn> {
        match self {
            Self::Background(BackgroundTaskBuilder { builder, .. }) => Some(builder(session)),
            Self::Sync(_) => None,
        }
    }
}
