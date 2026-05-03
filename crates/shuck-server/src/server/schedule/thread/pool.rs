use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam::channel::{Receiver, Sender};

use super::ThreadPriority;

pub(crate) struct Pool {
    job_sender: Sender<Job>,
    _handles: Vec<std::thread::JoinHandle<()>>,
    extant_tasks: Arc<AtomicUsize>,
}

struct Job {
    #[allow(dead_code)]
    requested_priority: ThreadPriority,
    f: Box<dyn FnOnce() + Send + 'static>,
}

impl Pool {
    pub(crate) fn new(threads: NonZeroUsize) -> Self {
        let threads = usize::from(threads);
        let (job_sender, job_receiver) = crossbeam::channel::bounded(std::cmp::min(threads * 2, 4));
        let extant_tasks = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::with_capacity(threads);
        for index in 0..threads {
            handles.push(spawn_worker(
                index,
                job_receiver.clone(),
                extant_tasks.clone(),
            ));
        }

        Self {
            job_sender,
            _handles: handles,
            extant_tasks,
        }
    }

    pub(crate) fn spawn<F>(&self, priority: ThreadPriority, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        if let Err(error) = self.job_sender.send(Job {
            requested_priority: priority,
            f: Box::new(f),
        }) {
            tracing::error!("Failed to dispatch background job: {error}");
        }
    }

    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.extant_tasks.load(Ordering::SeqCst)
    }
}

fn spawn_worker(
    index: usize,
    job_receiver: Receiver<Job>,
    extant_tasks: Arc<AtomicUsize>,
) -> std::thread::JoinHandle<()> {
    match std::thread::Builder::new()
        .name(format!("shuck:worker:{index}"))
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            for job in job_receiver {
                extant_tasks.fetch_add(1, Ordering::SeqCst);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job.f));
                extant_tasks.fetch_sub(1, Ordering::SeqCst);
                if let Err(error) = result {
                    if let Some(message) = error.downcast_ref::<String>() {
                        tracing::error!("Worker thread panicked with: {message}; aborting");
                    } else if let Some(message) = error.downcast_ref::<&str>() {
                        tracing::error!("Worker thread panicked with: {message}; aborting");
                    } else {
                        tracing::error!("Worker thread panicked; aborting");
                    }
                    std::process::abort();
                }
            }
        }) {
        Ok(handle) => handle,
        Err(error) => panic!("failed to spawn background worker thread: {error}"),
    }
}
