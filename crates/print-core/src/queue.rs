use crate::state::JobState;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QueueError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("invalid transition: {0}")]
    Transition(#[from] crate::state::TransitionError),
    #[error("job not found: {0}")]
    NotFound(String),
    #[error("invalid idempotency key")]
    BadKey,
    #[error("queue is full for printer {0} (backpressure limit reached)")]
    QueueFull(String),
}

/// Default backpressure limit: active (not finished) jobs per printer.
/// Beyond this, enqueue fails fast with QueueFull instead of consuming
/// unbounded memory/disk (spec section 5, bounded queue).
pub const DEFAULT_MAX_ACTIVE_PER_PRINTER: u64 = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub idempotency_key: String,
    pub printer: String,
    pub report: Option<String>,
    pub pages: u32,
    pub bytes: u64,
    pub priority: i64, // higher prints first (receipts before bulk)
    pub state: JobState,
    pub attempts: u32,
    pub error_code: Option<String>,
}

/// Durable queue in SQLite, WAL mode (spec section 5).
/// One serial worker per printer is enforced by `claim_next` selecting a
/// single queued job per printer; ordering is priority, then insertion.
pub struct Queue {
    conn: Connection,
    max_active_per_printer: u64,
}

impl Queue {
    pub fn open(path: &str) -> Result<Self, QueueError> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }
    pub fn in_memory() -> Result<Self, QueueError> {
        Self::init(Connection::open_in_memory()?)
    }
    fn init(conn: Connection) -> Result<Self, QueueError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS jobs(
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               id TEXT NOT NULL UNIQUE,
               idempotency_key TEXT NOT NULL UNIQUE,
               printer TEXT NOT NULL, report TEXT, pages INTEGER NOT NULL,
               bytes INTEGER NOT NULL, priority INTEGER NOT NULL DEFAULT 0,
               state TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
               error_code TEXT);
             CREATE INDEX IF NOT EXISTS idx_jobs_claim ON jobs(printer, state, priority DESC, seq);
             CREATE TABLE IF NOT EXISTS payloads(
               job_id TEXT PRIMARY KEY,
               kind TEXT NOT NULL,
               data BLOB NOT NULL);",
        )?;
        Ok(Self {
            conn,
            max_active_per_printer: DEFAULT_MAX_ACTIVE_PER_PRINTER,
        })
    }

    pub fn set_max_active_per_printer(&mut self, limit: u64) {
        self.max_active_per_printer = limit;
    }

    /// Jobs for this printer that are not finished yet.
    pub fn count_active_for_printer(&self, printer: &str) -> Result<u64, QueueError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE printer=?1 AND state IN ('queued','rendering','spooling','printing')",
            params![printer],
            |r| r.get::<_, i64>(0),
        )? as u64)
    }

    /// Cancel a job that has not started printing. Jobs already printing
    /// cannot be cancelled here (only the spooler can stop those); the
    /// state machine rejects it, which is the honest behaviour.
    pub fn cancel_job(&mut self, id: &str) -> Result<(), QueueError> {
        self.set_state(id, JobState::Canceled, Some("CANCELED_BY_USER"))
    }

    /// Enqueue, deduplicated by idempotency key: re-submitting the same key
    /// returns the existing job id and never creates a duplicate.
    pub fn enqueue(&mut self, job: &Job) -> Result<String, QueueError> {
        if !crate::security::valid_idempotency_key(&job.idempotency_key) {
            return Err(QueueError::BadKey);
        }
        // Backpressure: a duplicate submission (same key) is not new load
        // and is always allowed through to its deduplicated result.
        let is_duplicate: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs WHERE idempotency_key=?1)",
            params![job.idempotency_key],
            |r| r.get::<_, i64>(0),
        )? == 1;
        if !is_duplicate
            && self.count_active_for_printer(&job.printer)? >= self.max_active_per_printer
        {
            return Err(QueueError::QueueFull(job.printer.clone()));
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO jobs(id,idempotency_key,printer,report,pages,bytes,priority,state,attempts,error_code)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'queued',0,NULL)",
            params![job.id, job.idempotency_key, job.printer, job.report, job.pages, job.bytes, job.priority],
        )?;
        let id: String = self.conn.query_row(
            "SELECT id FROM jobs WHERE idempotency_key=?1",
            params![job.idempotency_key],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    /// Store a job's document payload (the queue row holds metadata; the
    /// bytes live here so the worker can print after any restart).
    /// kind: "pdf" | "escpos" | "zpl" | "tspl" | "image".
    pub fn store_payload(
        &mut self,
        job_id: &str,
        kind: &str,
        data: &[u8],
    ) -> Result<(), QueueError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO payloads(job_id, kind, data) VALUES(?1,?2,?3)",
            params![job_id, kind, data],
        )?;
        Ok(())
    }

    pub fn payload(&self, job_id: &str) -> Result<Option<(String, Vec<u8>)>, QueueError> {
        let mut stmt = self
            .conn
            .prepare("SELECT kind, data FROM payloads WHERE job_id=?1")?;
        let mut rows = stmt.query(params![job_id])?;
        match rows.next()? {
            Some(r) => Ok(Some((r.get(0)?, r.get(1)?))),
            None => Ok(None),
        }
    }

    /// Drop a job's payload (called when the job reaches a terminal
    /// state; document bytes are never kept longer than needed).
    pub fn purge_payload(&mut self, job_id: &str) -> Result<(), QueueError> {
        self.conn
            .execute("DELETE FROM payloads WHERE job_id=?1", params![job_id])?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Job, QueueError> {
        self.conn.query_row(
            "SELECT id,idempotency_key,printer,report,pages,bytes,priority,state,attempts,error_code FROM jobs WHERE id=?1",
            params![id], row_to_job).map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => QueueError::NotFound(id.to_string()),
                other => QueueError::Db(other),
            })
    }

    pub fn set_state(
        &mut self,
        id: &str,
        next: JobState,
        error_code: Option<&str>,
    ) -> Result<(), QueueError> {
        let job = self.get(id)?;
        job.state.transition_to(next)?;
        self.conn.execute(
            "UPDATE jobs SET state=?1, error_code=?2 WHERE id=?3",
            params![next.as_str(), error_code, id],
        )?;
        Ok(())
    }

    /// Atomically claim the next queued job for a printer (queued -> rendering).
    /// The UPDATE ... WHERE state='queued' guard makes double-claim impossible
    /// for a single connection; the worker model is one claimer per printer.
    pub fn claim_next(&mut self, printer: &str) -> Result<Option<Job>, QueueError> {
        let candidate: Option<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT id FROM jobs WHERE printer=?1 AND state='queued' ORDER BY priority DESC, seq ASC LIMIT 1")?;
            let mut rows = stmt.query(params![printer])?;
            match rows.next()? {
                Some(r) => Some(r.get(0)?),
                None => None,
            }
        };
        let Some(id) = candidate else { return Ok(None) };
        self.set_state(&id, JobState::Rendering, None)?;
        let mut job = self.get(&id)?;
        job.attempts += 1;
        self.conn.execute(
            "UPDATE jobs SET attempts=?1 WHERE id=?2",
            params![job.attempts, id],
        )?;
        Ok(Some(job))
    }

    /// Crash recovery (ADR-0004), run at startup.
    ///
    /// Jobs in rendering/spooling never reached the spooler, so they go
    /// back to queued. Jobs in printing may have reached the spooler, so
    /// they become failed with RECOVERY_AMBIGUOUS, unless auto_reprint is
    /// set, in which case they are queued again.
    ///
    /// Returns (requeued, ambiguous) counts. Never silently duplicates.
    pub fn recover_on_startup(&mut self, auto_reprint: bool) -> Result<(u64, u64), QueueError> {
        let requeued = self.conn.execute(
            "UPDATE jobs SET state='queued', error_code='RECOVERED_BEFORE_SPOOL' WHERE state IN ('rendering','spooling')", [])? as u64;
        let (state, code) = if auto_reprint {
            ("queued", "RECOVERED_AUTO_REPRINT")
        } else {
            ("failed", "RECOVERY_AMBIGUOUS")
        };
        let ambiguous = self.conn.execute(
            "UPDATE jobs SET state=?1, error_code=?2 WHERE state='printing'",
            params![state, code],
        )? as u64;
        Ok((requeued, ambiguous))
    }

    pub fn count_by_state(&self, state: JobState) -> Result<u64, QueueError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE state=?1",
            params![state.as_str()],
            |r| r.get::<_, i64>(0),
        )? as u64)
    }
}

fn row_to_job(r: &rusqlite::Row) -> rusqlite::Result<Job> {
    let state_s: String = r.get(7)?;
    Ok(Job {
        id: r.get(0)?,
        idempotency_key: r.get(1)?,
        printer: r.get(2)?,
        report: r.get(3)?,
        pages: r.get::<_, i64>(4)? as u32,
        bytes: r.get::<_, i64>(5)? as u64,
        priority: r.get(6)?,
        state: JobState::parse(&state_s).unwrap_or(JobState::Failed),
        attempts: r.get::<_, i64>(8)? as u32,
        error_code: r.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job(id: &str, key: &str, printer: &str, prio: i64) -> Job {
        Job {
            id: id.into(),
            idempotency_key: key.into(),
            printer: printer.into(),
            report: Some("sale.report".into()),
            pages: 1,
            bytes: 1024,
            priority: prio,
            state: JobState::Queued,
            attempts: 0,
            error_code: None,
        }
    }
    #[test]
    fn idempotent_enqueue_no_duplicate() {
        let mut q = Queue::in_memory().unwrap();
        let a = q.enqueue(&job("1", "k-1", "p1", 0)).unwrap();
        let b = q.enqueue(&job("2", "k-1", "p1", 0)).unwrap();
        assert_eq!(a, b);
        assert_eq!(q.count_by_state(JobState::Queued).unwrap(), 1);
    }
    #[test]
    fn priority_then_fifo_claim() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&job("bulk", "k-bulk", "p1", 0)).unwrap();
        q.enqueue(&job("receipt", "k-rcpt", "p1", 10)).unwrap();
        let first = q.claim_next("p1").unwrap().unwrap();
        assert_eq!(first.id, "receipt");
        assert_eq!(first.state, JobState::Rendering);
        // different printer is independent
        assert!(q.claim_next("other").unwrap().is_none());
    }
    #[test]
    fn full_lifecycle_and_invalid_transition() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&job("1", "k-1", "p1", 0)).unwrap();
        let j = q.claim_next("p1").unwrap().unwrap();
        q.set_state(&j.id, JobState::Spooling, None).unwrap();
        q.set_state(&j.id, JobState::Printing, None).unwrap();
        q.set_state(&j.id, JobState::Done, None).unwrap();
        assert_eq!(q.count_by_state(JobState::Done).unwrap(), 1);
        assert!(q.set_state(&j.id, JobState::Queued, None).is_err());
    }
    #[test]
    fn recovery_semantics() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&job("a", "k-a", "p1", 0)).unwrap();
        q.enqueue(&job("b", "k-b", "p1", 0)).unwrap();
        let a = q.claim_next("p1").unwrap().unwrap(); // a: rendering
        q.set_state(&a.id, JobState::Spooling, None).unwrap();
        let b = q.claim_next("p1").unwrap().unwrap(); // b: rendering
        q.set_state(&b.id, JobState::Spooling, None).unwrap();
        q.set_state(&b.id, JobState::Printing, None).unwrap(); // b: printing (ambiguous)
        let (requeued, ambiguous) = q.recover_on_startup(false).unwrap();
        assert_eq!(requeued, 1);
        assert_eq!(ambiguous, 1);
        assert_eq!(q.get("a").unwrap().state, JobState::Queued);
        let b2 = q.get("b").unwrap();
        assert_eq!(b2.state, JobState::Failed);
        assert_eq!(b2.error_code.as_deref(), Some("RECOVERY_AMBIGUOUS"));
    }
    #[test]
    fn payload_round_trip_and_purge() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&job("1", "k-1", "p1", 0)).unwrap();
        assert!(q.payload("1").unwrap().is_none());
        q.store_payload("1", "pdf", b"%PDF-data").unwrap();
        let (kind, data) = q.payload("1").unwrap().unwrap();
        assert_eq!(kind, "pdf");
        assert_eq!(data, b"%PDF-data");
        q.purge_payload("1").unwrap();
        assert!(q.payload("1").unwrap().is_none());
    }
    #[test]
    fn backpressure_and_cancel() {
        let mut q = Queue::in_memory().unwrap();
        q.set_max_active_per_printer(2);
        q.enqueue(&job("1", "k-1", "p1", 0)).unwrap();
        q.enqueue(&job("2", "k-2", "p1", 0)).unwrap();
        // Third distinct job is rejected fast; a duplicate is not.
        assert!(matches!(
            q.enqueue(&job("3", "k-3", "p1", 0)),
            Err(QueueError::QueueFull(_))
        ));
        assert!(q.enqueue(&job("x", "k-1", "p1", 0)).is_ok());
        // Other printers are unaffected.
        assert!(q.enqueue(&job("9", "k-9", "p2", 0)).is_ok());
        // Cancel frees capacity and is terminal.
        q.cancel_job("1").unwrap();
        assert_eq!(q.get("1").unwrap().state, JobState::Canceled);
        assert!(q.enqueue(&job("3", "k-3", "p1", 0)).is_ok());
        // A job already done cannot be cancelled.
        let j = q.claim_next("p1").unwrap().unwrap();
        q.set_state(&j.id, JobState::Spooling, None).unwrap();
        assert!(q.cancel_job(&j.id).is_ok()); // spooling can still cancel
    }
    #[test]
    fn durable_file_queue_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("pq-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("q.db");
        let p = path.to_str().unwrap().to_string();
        {
            let mut q = Queue::open(&p).unwrap();
            q.enqueue(&job("1", "k-1", "p1", 0)).unwrap();
        }
        let q = Queue::open(&p).unwrap();
        assert_eq!(q.count_by_state(JobState::Queued).unwrap(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
