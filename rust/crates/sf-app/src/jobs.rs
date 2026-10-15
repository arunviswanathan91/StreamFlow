//! Run heavy computations on a worker thread so the interface stays responsive. The UI polls each frame.

use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

pub struct Job<T> {
    pub label: String,
    rx: Receiver<Result<T, String>>,
    started: Instant,
}

/// Start `f` on a new thread. A panic in `f` is reported as an error, never as a hang.
pub fn spawn<T: Send + 'static>(label: impl Into<String>, f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Job<T> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|p| {
            let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown panic".into());
            Err(format!("the computation crashed: {msg}"))
        });
        let _ = tx.send(out);
    });
    Job { label: label.into(), rx, started: Instant::now() }
}

impl<T> Job<T> {
    /// `None` while running; the result exactly once when finished.
    pub fn poll(&mut self) -> Option<Result<T, String>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("the worker thread ended without a result".into())),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Block until done (tests and scripts).
    pub fn wait(mut self) -> Result<T, String> {
        loop {
            if let Some(r) = self.poll() {
                return r;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_errors_and_panics_all_come_back() {
        assert_eq!(spawn("ok", || Ok(41 + 1)).wait(), Ok(42));
        assert_eq!(spawn::<u8>("err", || Err("nope".into())).wait(), Err("nope".into()));
        let e = spawn::<u8>("boom", || panic!("kaboom")).wait().unwrap_err();
        assert!(e.contains("crashed") && e.contains("kaboom"), "{e}");
    }

    #[test]
    fn poll_is_none_while_running_and_a_result_arrives_once() {
        let mut j = spawn("slow", || {
            std::thread::sleep(Duration::from_millis(80));
            Ok(7)
        });
        assert!(j.poll().is_none());
        assert!(j.elapsed() < Duration::from_millis(80));
        let r = loop {
            if let Some(r) = j.poll() {
                break r;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(r, Ok(7));
        assert!(matches!(j.poll(), Some(Err(_))), "after the result, the channel is closed");
    }
}
