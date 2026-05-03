use std::num::NonZeroUsize;

use crate::session::{Client, Session};

mod task;
mod thread;

pub(super) use task::{BackgroundSchedule, Task};

use self::{
    task::{BackgroundTaskBuilder, SyncTask},
    thread::{Pool, ThreadPriority},
};

pub(crate) fn spawn_main_loop(
    func: impl FnOnce() -> crate::Result<()> + Send + 'static,
) -> crate::Result<std::thread::JoinHandle<crate::Result<()>>> {
    const MAIN_THREAD_STACK_SIZE: usize = 2 * 1024 * 1024;
    Ok(std::thread::Builder::new()
        .name("shuck:main".into())
        .stack_size(MAIN_THREAD_STACK_SIZE)
        .spawn(func)?)
}

pub(crate) struct Scheduler {
    fmt_pool: Pool,
    background_pool: Pool,
}

impl Scheduler {
    pub(super) fn new(worker_threads: NonZeroUsize) -> Self {
        Self {
            fmt_pool: Pool::new(NonZeroUsize::MIN),
            background_pool: Pool::new(worker_threads),
        }
    }

    pub(super) fn dispatch(&mut self, task: Task, session: &mut Session, client: Client) {
        match task {
            Task::Sync(SyncTask { func }) => func(session, &client),
            Task::Background(BackgroundTaskBuilder { schedule, builder }) => {
                let static_func = builder(session);
                let task = move || static_func(&client);
                match schedule {
                    BackgroundSchedule::Fmt => {
                        self.fmt_pool.spawn(ThreadPriority::LatencySensitive, task);
                    }
                    BackgroundSchedule::LatencySensitive => {
                        self.background_pool
                            .spawn(ThreadPriority::LatencySensitive, task);
                    }
                    BackgroundSchedule::Worker => {
                        self.background_pool.spawn(ThreadPriority::Worker, task);
                    }
                }
            }
        }
    }
}
