use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

type Job = Box<dyn FnOnce() + Send + 'static>;

pub(super) struct RequestPool {
    sender: Option<mpsc::Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
}

impl RequestPool {
    pub(super) fn new(name: &str, worker_count: usize) -> std::io::Result<Self> {
        assert!(
            worker_count > 0,
            "request pool must have at least one worker"
        );

        let (sender, receiver) = mpsc::channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(worker_count);

        for index in 0..worker_count {
            let receiver = Arc::clone(&receiver);
            let worker = thread::Builder::new()
                .name(format!("maki-{name}-worker-{index}"))
                .spawn(move || worker_loop(&receiver))?;
            workers.push(worker);
        }

        Ok(Self {
            sender: Some(sender),
            workers,
        })
    }

    pub(super) fn execute(&self, job: impl FnOnce() + Send + 'static) {
        let Some(sender) = &self.sender else {
            return;
        };

        if sender.send(Box::new(job)).is_err() {
            eprintln!("Request worker pool stopped before accepting a connection");
        }
    }
}

fn worker_loop(receiver: &Mutex<mpsc::Receiver<Job>>) {
    loop {
        let job = {
            let receiver = receiver.lock().unwrap_or_else(|error| error.into_inner());
            receiver.recv()
        };

        let Ok(job) = job else {
            return;
        };

        if panic::catch_unwind(AssertUnwindSafe(job)).is_err() {
            eprintln!("Request worker recovered after a panicked job");
        }
    }
}

impl Drop for RequestPool {
    fn drop(&mut self) {
        self.sender.take();

        for worker in self.workers.drain(..) {
            if worker.join().is_err() {
                eprintln!("Request worker panicked while shutting down");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RequestPool;
    use std::collections::HashSet;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn fixed_workers_are_reused_across_jobs() {
        let pool = RequestPool::new("test", 2).unwrap();
        let (sender, receiver) = mpsc::channel();
        let mut releases = Vec::new();

        for _ in 0..2 {
            let sender = sender.clone();
            let (release, released) = mpsc::channel();
            releases.push(release);
            pool.execute(move || {
                sender.send(thread::current().id()).unwrap();
                let _ = released.recv();
            });
        }

        let worker_ids = (0..2)
            .map(|_| receiver.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect::<HashSet<_>>();
        assert_eq!(worker_ids.len(), 2);
        for release in releases {
            release.send(()).unwrap();
        }

        for _ in 0..8 {
            let sender = sender.clone();
            pool.execute(move || sender.send(thread::current().id()).unwrap());
        }

        for _ in 0..8 {
            let worker_id = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(worker_ids.contains(&worker_id));
        }
    }
}
