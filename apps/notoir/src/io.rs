//! Serialized background IO (docs/specs/01-architecture.md §Async): small
//! writes (settings files, state DB rows) run in order on one thread so
//! read-modify-write sequences never race, and the UI thread never waits on
//! the disk. Results come back as messages drained once per frame.
//!
//! Larger async work (scans, search, Git) gets the Tokio worker pool when it
//! arrives (M5/M7); this queue stays for ordered writes.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

/// Outcome of an IO job that the UI must react to.
#[derive(Debug)]
pub enum IoResult {
    SettingsWriteFailed { path: String, error: String },
}

type Job = Box<dyn FnOnce() -> Option<IoResult> + Send>;

pub struct IoQueue {
    jobs: Sender<Job>,
    results: Receiver<IoResult>,
    pending: Arc<AtomicUsize>,
}

impl IoQueue {
    /// Starts the IO thread; `wake` is called after every job (request a repaint).
    pub fn start(wake: impl Fn() + Send + 'static) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (result_tx, results) = channel();
        let pending = Arc::new(AtomicUsize::new(0));
        let counter = pending.clone();
        let spawned = std::thread::Builder::new()
            .name("notoir-io".into())
            .spawn(move || {
                for job in job_rx {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                    match result {
                        Ok(Some(r)) => {
                            let _ = result_tx.send(r);
                        }
                        Ok(None) => {}
                        Err(_) => tracing::error!("IO job panicked"),
                    }
                    counter.fetch_sub(1, Ordering::SeqCst);
                    wake();
                }
            });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "could not start IO thread");
        }
        Self {
            jobs,
            results,
            pending,
        }
    }

    pub fn submit(&self, job: impl FnOnce() -> Option<IoResult> + Send + 'static) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self.jobs.send(Box::new(job)).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            tracing::error!("IO thread is gone; job dropped");
        }
    }

    /// Results ready now (non-blocking).
    pub fn drain(&self) -> Vec<IoResult> {
        self.results.try_iter().collect()
    }

    /// Blocks until queued jobs finished (tests, shutdown). Returns false on timeout.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        while self.pending.load(Ordering::SeqCst) > 0 {
            if start.elapsed() > timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn jobs_run_in_submission_order() {
        let q = IoQueue::start(|| {});
        let log = Arc::new(Mutex::new(Vec::new()));
        for i in 0..20 {
            let log = log.clone();
            q.submit(move || {
                log.lock().unwrap().push(i);
                None
            });
        }
        assert!(q.wait_idle(Duration::from_secs(5)));
        assert_eq!(*log.lock().unwrap(), (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn results_and_panics() {
        let q = IoQueue::start(|| {});
        q.submit(|| panic!("boom"));
        q.submit(|| {
            Some(IoResult::SettingsWriteFailed {
                path: "x".into(),
                error: "denied".into(),
            })
        });
        assert!(q.wait_idle(Duration::from_secs(5)));
        assert_eq!(
            q.drain().len(),
            1,
            "a panicking job does not stop the queue"
        );
    }
}
