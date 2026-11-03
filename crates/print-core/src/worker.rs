use crate::queue::{Job, Queue, QueueError};
use crate::state::JobState;
use thiserror::Error;

/// A platform print backend: Windows GDI, Windows RAW spooler, TCP 9100,
/// IPP, AirPrint. The worker owns job lifecycle; backends only print.
pub trait PrintBackend {
    /// Print one claimed job. On success the job's bytes reached the
    /// printer/spooler. Errors carry a stable internal error code.
    fn print(&mut self, job: &Job) -> Result<(), BackendError>;
}

#[derive(Debug, Error)]
#[error("{code}: {message}")]
pub struct BackendError {
    pub code: String,
    pub message: String,
    /// Retryable errors (offline, busy, network drop) go back to the queue;
    /// fatal errors (bad document, wrong paper) fail immediately.
    pub retryable: bool,
}

impl BackendError {
    pub fn retryable(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: true,
        }
    }
    pub fn fatal(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WorkerConfig {
    pub max_attempts: u32,
    pub backoff_base_ms: u64,
    pub backoff_max_ms: u64,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            backoff_base_ms: 500,
            backoff_max_ms: 30_000,
        }
    }
}

/// Exponential backoff with deterministic jitter (seeded by attempt and a
/// caller-supplied entropy value, so tests are reproducible):
/// delay = min(max, base * 2^attempt) + jitter in [0, base).
pub fn backoff_delay_ms(cfg: &WorkerConfig, attempt: u32, entropy: u64) -> u64 {
    let exp = cfg
        .backoff_base_ms
        .saturating_mul(2u64.saturating_pow(attempt.min(20)));
    let capped = exp.min(cfg.backoff_max_ms);
    let jitter = if cfg.backoff_base_ms == 0 {
        0
    } else {
        entropy % cfg.backoff_base_ms
    };
    capped.saturating_add(jitter)
}

/// Process jobs for one printer serially (one worker per printer, spec
/// section 5). `run_once` claims and handles at most one job; the caller
/// (app shell) loops it. Sleeper is injected so tests never really sleep.
pub struct Worker<'a, B: PrintBackend, S: FnMut(u64)> {
    pub queue: &'a mut Queue,
    pub backend: B,
    pub cfg: WorkerConfig,
    pub sleep: S,
    pub entropy: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Idle,
    Done(String),
    Retried(String),
    Failed(String),
    DeadLetter(String),
}

impl<'a, B: PrintBackend, S: FnMut(u64)> Worker<'a, B, S> {
    pub fn run_once(&mut self, printer: &str) -> Result<Outcome, QueueError> {
        let Some(job) = self.queue.claim_next(printer)? else {
            return Ok(Outcome::Idle);
        };
        // Walk the state machine to the spooler step before printing, so a
        // crash during backend.print() leaves the job in spooling/printing
        // (the recovery rules in ADR-0004 distinguish exactly this).
        self.queue.set_state(&job.id, JobState::Spooling, None)?;
        self.queue.set_state(&job.id, JobState::Printing, None)?;
        match self.backend.print(&job) {
            Ok(()) => {
                self.queue.set_state(&job.id, JobState::Done, None)?;
                self.queue.purge_payload(&job.id)?;
                Ok(Outcome::Done(job.id))
            }
            Err(e) if e.retryable && job.attempts < self.cfg.max_attempts => {
                // Back to failed, then re-queue is a separate explicit
                // transition (Failed -> Queued) so the audit trail shows it.
                self.queue
                    .set_state(&job.id, JobState::Failed, Some(&e.code))?;
                self.queue
                    .set_state(&job.id, JobState::Queued, Some(&e.code))?;
                let delay = backoff_delay_ms(&self.cfg, job.attempts, self.entropy);
                (self.sleep)(delay);
                Ok(Outcome::Retried(job.id))
            }
            Err(e) if e.retryable => {
                self.queue
                    .set_state(&job.id, JobState::Failed, Some(&e.code))?;
                self.queue
                    .set_state(&job.id, JobState::DeadLetter, Some(&e.code))?;
                // Payload is kept for a dead-letter job on purpose: the
                // one-click retry needs the document. It is purged only
                // when the retried job eventually finishes.
                Ok(Outcome::DeadLetter(job.id))
            }
            Err(e) => {
                self.queue
                    .set_state(&job.id, JobState::Failed, Some(&e.code))?;
                Ok(Outcome::Failed(job.id))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct ScriptedBackend {
        results: VecDeque<Result<(), BackendError>>,
    }
    impl PrintBackend for ScriptedBackend {
        fn print(&mut self, _job: &Job) -> Result<(), BackendError> {
            self.results.pop_front().unwrap_or(Ok(()))
        }
    }
    fn job(id: &str) -> Job {
        Job {
            id: id.into(),
            idempotency_key: format!("k-{id}"),
            printer: "p1".into(),
            report: None,
            pages: 1,
            bytes: 10,
            priority: 0,
            state: JobState::Queued,
            attempts: 0,
            error_code: None,
        }
    }
    #[test]
    fn backoff_grows_and_caps() {
        let cfg = WorkerConfig {
            max_attempts: 4,
            backoff_base_ms: 500,
            backoff_max_ms: 2_000,
        };
        assert_eq!(backoff_delay_ms(&cfg, 0, 0), 500);
        assert_eq!(backoff_delay_ms(&cfg, 1, 0), 1_000);
        assert_eq!(backoff_delay_ms(&cfg, 10, 0), 2_000);
        assert!(backoff_delay_ms(&cfg, 0, 250) >= 500);
    }
    #[test]
    fn success_path() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&job("1")).unwrap();
        q.store_payload("1", "pdf", b"%PDF-1.4").unwrap();
        let mut w = Worker {
            queue: &mut q,
            backend: ScriptedBackend {
                results: VecDeque::from([Ok(())]),
            },
            cfg: WorkerConfig::default(),
            sleep: |_| {},
            entropy: 0,
        };
        assert_eq!(w.run_once("p1").unwrap(), Outcome::Done("1".into()));
        drop(w);
        assert_eq!(q.count_by_state(JobState::Done).unwrap(), 1);
        assert_eq!(q.count_by_state(JobState::Queued).unwrap(), 0);
        assert!(q.payload("1").unwrap().is_none());
    }
    #[test]
    fn retry_then_dead_letter() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&job("1")).unwrap();
        let cfg = WorkerConfig {
            max_attempts: 2,
            backoff_base_ms: 1,
            backoff_max_ms: 5,
        };
        let mut slept = Vec::new();
        {
            let mut w = Worker {
                queue: &mut q,
                backend: ScriptedBackend {
                    results: VecDeque::from([
                        Err(BackendError::retryable("PRINTER_OFFLINE", "offline")),
                        Err(BackendError::retryable("PRINTER_OFFLINE", "offline")),
                    ]),
                },
                cfg,
                sleep: |d| slept.push(d),
                entropy: 0,
            };
            // Attempt 1 (attempts becomes 1 at claim): retryable -> re-queued.
            assert_eq!(w.run_once("p1").unwrap(), Outcome::Retried("1".into()));
            // Attempt 2: attempts == max -> dead letter.
            assert_eq!(w.run_once("p1").unwrap(), Outcome::DeadLetter("1".into()));
        }
        assert_eq!(slept.len(), 1);
        assert_eq!(q.count_by_state(JobState::DeadLetter).unwrap(), 1);
        // One-click retry from dead letter works.
        q.set_state("1", JobState::Queued, None).unwrap();
        assert_eq!(q.count_by_state(JobState::Queued).unwrap(), 1);
    }
    #[test]
    fn fatal_fails_without_retry() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&job("1")).unwrap();
        let mut w = Worker {
            queue: &mut q,
            backend: ScriptedBackend {
                results: VecDeque::from([Err(BackendError::fatal("PAPER_MISMATCH", "wrong size"))]),
            },
            cfg: WorkerConfig::default(),
            sleep: |_| {},
            entropy: 0,
        };
        assert_eq!(w.run_once("p1").unwrap(), Outcome::Failed("1".into()));
        drop(w);
        let j = q.get("1").unwrap();
        assert_eq!(j.error_code.as_deref(), Some("PAPER_MISMATCH"));
    }
}
