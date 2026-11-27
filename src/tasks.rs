//! A bounded pool for server requests. Navigation, artwork, health checks and saves queue
//! here instead of each starting a thread. Sign-in polling and playback keep their own
//! threads because they run for minutes.

use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError},
    thread,
    time::Duration,
};

/// Enough for a page, its artwork and a few servers' refreshes at once. A request that
/// hangs until its timeout holds one worker, so the rest of the app stays responsive.
const MAX_WORKERS: usize = 12;
const IDLE_EXIT: Duration = Duration::from_secs(30);

type Job = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    idle: usize,
    workers: usize,
}

#[derive(Default)]
struct Pool {
    queue: Mutex<Queue>,
    ready: Condvar,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(Pool::default)
}

fn lock(pool: &Pool) -> MutexGuard<'_, Queue> {
    pool.queue.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs `job` on a pooled worker. Jobs that belong to a view should check its generation
/// first and return early once the user has moved on.
pub fn spawn(job: impl FnOnce() + Send + 'static) {
    let pool = pool();
    let mut queue = lock(pool);
    queue.jobs.push_back(Box::new(job));
    if queue.jobs.len() > queue.idle && queue.workers < MAX_WORKERS {
        queue.workers += 1;
        let started = thread::Builder::new()
            .name("perfuse-request".into())
            .spawn(work);
        if started.is_err() {
            queue.workers -= 1;
        }
    }
    drop(queue);
    pool.ready.notify_one();
}

fn work() {
    let pool = pool();
    loop {
        let job = {
            let mut queue = lock(pool);
            loop {
                if let Some(job) = queue.jobs.pop_front() {
                    break job;
                }
                queue.idle += 1;
                let (next, wait) = pool
                    .ready
                    .wait_timeout(queue, IDLE_EXIT)
                    .unwrap_or_else(PoisonError::into_inner);
                queue = next;
                queue.idle -= 1;
                if wait.timed_out() && queue.jobs.is_empty() {
                    queue.workers -= 1;
                    return;
                }
            }
        };
        // A panicking job must not take the worker, or its count, with it.
        let _ = catch_unwind(AssertUnwindSafe(job));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    #[test]
    fn runs_every_job_with_bounded_workers_and_survives_panics() {
        let done = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel();
        spawn(|| panic!("a failing job"));
        for _ in 0..40 {
            let (done, tx) = (done.clone(), tx.clone());
            spawn(move || {
                thread::sleep(Duration::from_millis(5));
                done.fetch_add(1, Ordering::Relaxed);
                let _ = tx.send(());
            });
        }
        for _ in 0..40 {
            rx.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        assert_eq!(done.load(Ordering::Relaxed), 40);
        assert!(lock(pool()).workers <= MAX_WORKERS);
    }
}
