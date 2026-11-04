use crate::state::JobState;
use rusqlite::{params, Connection, OptionalExtension};
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
    #[error("stored job options are unreadable: {0}")]
    BadOptions(String),
    #[error("payload exceeds the {0}-byte limit")]
    PayloadTooLarge(usize),
}

/// Default backpressure limit: active (not finished) jobs per printer.
/// Beyond this, enqueue fails fast with QueueFull instead of consuming
/// unbounded memory/disk (spec section 5, bounded queue).
pub const DEFAULT_MAX_ACTIVE_PER_PRINTER: u64 = 500;

/// Global backpressure limit: active jobs across ALL printers.
/// Bounds total disk/memory a runaway client can consume.
pub const DEFAULT_MAX_PENDING_GLOBAL: u64 = 2000;

/// A job in a working state (rendering/spooling/printing) whose last
/// update is older than this is wedged, not working: the worker is
/// gone or stuck. Reconciliation fails it loudly (RECONCILED_STALE)
/// instead of letting it hang forever.
pub const STALE_JOB_THRESHOLD_SECS: i64 = 900;

/// Diagnostics correlation id for a job: a stable, short fingerprint
/// of its idempotency key. Safe to log and quote in support reports
/// (one-way fingerprint, not the key itself).
pub fn diag_id_for_key(key: &str) -> String {
    // FNV-1a 64-bit: deterministic across runs and platforms.
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in key.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("np-{hash:016x}")
}

/// FNV-1a 64-bit fingerprint of a payload, stored beside the blob so
/// duplicate detection never has to load the stored bytes (audit M3).
/// A collision only weakens a diagnostic log line, never a decision:
/// the stored job wins regardless of the comparison outcome.
fn payload_fingerprint(data: &[u8]) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in data {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as i64
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// One structured job event (the durable diagnostics log).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobEvent {
    pub job_id: String,
    pub ts: i64,
    pub event: String,
    pub detail: Option<String>,
}

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

/// Execution options stored alongside a queued job so the background
/// worker knows the profile's mode, paper, copies, scale policy, and
/// colour settings even after an app restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobOptions {
    pub mode: String,
    pub paper: Option<String>,
    pub copies: u16,
    pub scale_policy: Option<String>,
    pub duplex: bool,
    pub color: bool,
    pub cut: bool,
    #[serde(default)]
    pub tray: Option<String>,
    /// Receipt roll width (mm) reported by the server from the
    /// printer's driver forms. Lets network (TCP/IPP) printers fit
    /// output to their real media even with no local Windows driver.
    #[serde(default)]
    pub media_roll_width_mm: Option<f64>,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            mode: "gdi".to_string(),
            paper: None,
            copies: 1,
            scale_policy: Some("custom_allowed".to_string()),
            duplex: false,
            color: true,
            cut: true,
            tray: None,
            media_roll_width_mm: None,
        }
    }
}

/// Durable queue in SQLite, WAL mode (spec section 5).
/// One serial worker per printer is enforced by `claim_next` selecting a
/// single queued job per printer; ordering is priority, then insertion.
pub struct Queue {
    conn: Connection,
    max_active_per_printer: u64,
    max_pending_global: u64,
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
        // 30 s, not 5: the command path and the worker hold separate
        // connections to the same database, and a burst (worker
        // mid-transaction while the user retries/cancels in the
        // Control Center) must resolve as a wait, not a user-visible
        // SQLITE_BUSY failure (audit L6).
        conn.busy_timeout(std::time::Duration::from_millis(30_000))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // FULL, not NORMAL: with WAL + NORMAL an OS crash or power
        // loss can roll back the last commits — for a print queue that
        // either loses an acknowledged job or resurrects a printed one
        // (a duplicate). Enqueue rates here are human-scale, so the
        // stronger durability costs nothing measurable (see bench).
        conn.pragma_update(None, "synchronous", "FULL")?;
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
               data BLOB NOT NULL);
             CREATE TABLE IF NOT EXISTS job_options(
               job_id TEXT PRIMARY KEY,
               options_json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS job_events(
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               job_id TEXT NOT NULL,
               ts INTEGER NOT NULL,
               event TEXT NOT NULL,
               detail TEXT);",
        )?;
        // Schema version marker: future migrations key off this
        // instead of probing ad-hoc. 1 = the current schema.
        let user_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if user_version == 0 {
            conn.pragma_update(None, "user_version", 1)?;
        }
        // Migration: jobs.updated_at (unix seconds of the last state
        // change) powers stale-job reconciliation. Added in place so
        // existing queue databases upgrade without data loss.
        let has_updated_at: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('jobs') WHERE name='updated_at')",
            [],
            |r| r.get::<_, i64>(0),
        )? == 1;
        if !has_updated_at {
            conn.execute(
                "ALTER TABLE jobs ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        // Migration: payloads.payload_hash (FNV-1a 64 of the stored
        // bytes) lets duplicate-submission conflict detection compare
        // a hash instead of loading the whole stored blob (audit M3).
        // NULL on rows written before this migration: those fall back
        // to a length-only comparison.
        let has_payload_hash: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('payloads') WHERE name='payload_hash')",
            [],
            |r| r.get::<_, i64>(0),
        )? == 1;
        if !has_payload_hash {
            conn.execute("ALTER TABLE payloads ADD COLUMN payload_hash INTEGER", [])?;
        }
        Ok(Self {
            conn,
            max_active_per_printer: DEFAULT_MAX_ACTIVE_PER_PRINTER,
            max_pending_global: DEFAULT_MAX_PENDING_GLOBAL,
        })
    }

    pub fn set_max_active_per_printer(&mut self, limit: u64) {
        self.max_active_per_printer = limit;
    }

    pub fn set_max_pending_global(&mut self, limit: u64) {
        self.max_pending_global = limit;
    }

    /// Active (not finished) jobs across all printers.
    pub fn count_active_global(&self) -> Result<u64, QueueError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE state IN ('queued','rendering','spooling','printing')",
            [],
            |r| r.get::<_, i64>(0),
        )? as u64)
    }

    fn record_event(
        &self,
        job_id: &str,
        event: &str,
        detail: Option<&str>,
    ) -> Result<(), QueueError> {
        self.conn.execute(
            "INSERT INTO job_events(job_id, ts, event, detail) VALUES(?1,?2,?3,?4)",
            params![job_id, now_unix(), event, detail],
        )?;
        Ok(())
    }

    /// record_event for call sites that cannot fail their caller (state
    /// transitions, recovery): a dropped event write used to vanish
    /// silently (audit L3) — exactly when the disk is full and the
    /// history matters most. Log the failure to stderr instead.
    fn record_event_best_effort(&self, job_id: &str, event: &str, detail: Option<&str>) {
        if let Err(e) = self.record_event(job_id, event, detail) {
            eprintln!("native-print: failed to record job event {event} for {job_id}: {e}");
        }
    }

    /// The most recent structured job events (the diagnostics log).
    pub fn recent_events(&self, limit: usize) -> Result<Vec<JobEvent>, QueueError> {
        let mut stmt = self.conn.prepare(
            "SELECT job_id, ts, event, detail FROM job_events ORDER BY seq DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(JobEvent {
                job_id: r.get(0)?,
                ts: r.get(1)?,
                event: r.get(2)?,
                detail: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
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
    /// Payload bytes are purged on cancellation so canceled jobs do not leak disk space.
    pub fn cancel_job(&mut self, id: &str) -> Result<(), QueueError> {
        self.set_state(id, JobState::Canceled, Some("CANCELED_BY_USER"))?;
        self.purge_payload(id)?;
        Ok(())
    }

    /// Retry a failed or dead-letter job. The attempt counter is NOT
    /// reset: attempts are monotonic across the job's whole life (the
    /// audit invariant), so max-attempts accounting and diagnostics
    /// stay truthful across manual retries.
    pub fn retry_job(&mut self, id: &str) -> Result<(), QueueError> {
        self.set_state(id, JobState::Queued, None)?;
        self.conn
            .execute("UPDATE jobs SET error_code=NULL WHERE id=?1", params![id])?;
        Ok(())
    }

    /// Remove completed and canceled jobs (and their payloads/options).
    pub fn clear_finished(&mut self) -> Result<u64, QueueError> {
        self.conn.execute(
            "DELETE FROM payloads WHERE job_id IN (SELECT id FROM jobs WHERE state IN ('done','canceled'))",
            [],
        )?;
        self.conn.execute(
            "DELETE FROM job_options WHERE job_id IN (SELECT id FROM jobs WHERE state IN ('done','canceled'))",
            [],
        )?;
        self.conn.execute(
            "DELETE FROM job_events WHERE job_id IN (SELECT id FROM jobs WHERE state IN ('done','canceled'))",
            [],
        )?;
        let removed = self.conn.execute(
            "DELETE FROM jobs WHERE state IN ('done','canceled') AND 1=1",
            [],
        )? as u64;
        Ok(removed)
    }

    /// Distinct printer patterns that currently have queued jobs, ordered by
    /// highest priority and earliest arrival.
    pub fn queued_printers(&self) -> Result<Vec<String>, QueueError> {
        let mut stmt = self.conn.prepare(
            "SELECT printer FROM jobs WHERE state='queued' GROUP BY printer ORDER BY MAX(priority) DESC, MIN(seq) ASC",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// List the most recent jobs (newest first) up to `limit`.
    pub fn list_recent(&self, limit: usize) -> Result<Vec<Job>, QueueError> {
        let mut stmt = self.conn.prepare(
            "SELECT id,idempotency_key,printer,report,pages,bytes,priority,state,attempts,error_code FROM jobs ORDER BY seq DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], row_to_job)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
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
        if !is_duplicate && self.count_active_global()? >= self.max_pending_global {
            return Err(QueueError::QueueFull("global".to_string()));
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO jobs(id,idempotency_key,printer,report,pages,bytes,priority,state,attempts,error_code,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'queued',0,NULL,?8)",
            params![job.id, job.idempotency_key, job.printer, job.report, job.pages, job.bytes, job.priority, now_unix()],
        )?;
        if !is_duplicate {
            self.record_event_best_effort(
                &job.id,
                "enqueued",
                Some(&diag_id_for_key(&job.idempotency_key)),
            );
        }
        let id: String = self.conn.query_row(
            "SELECT id FROM jobs WHERE idempotency_key=?1",
            params![job.idempotency_key],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    /// Like `enqueue`, but reports whether the job was newly inserted.
    /// Returns `Ok(Some(id))` for a new job and `Ok(None)` when the
    /// idempotency key already exists: a duplicate submission must never
    /// overwrite the original job's payload or options (the worker may
    /// already be printing them).
    pub fn enqueue_new(&mut self, job: &Job) -> Result<Option<String>, QueueError> {
        let existed = self.job_id_for_key(&job.idempotency_key)?.is_some();
        let id = self.enqueue(job)?;
        if existed {
            Ok(None)
        } else {
            Ok(Some(id))
        }
    }

    /// Atomic enqueue: job row + payload + options are written in a
    /// SINGLE SQLite transaction. Either the complete job exists or
    /// nothing does — a crash or power loss mid-submission can never
    /// leave a queued job without its document (which the worker
    /// would then fail) or a payload without a job. Returns
    /// Ok(Some(id)) for a new job, Ok(None) for a duplicate key
    /// (nothing written; the original job is untouched).
    pub fn enqueue_with_payload(
        &mut self,
        job: &Job,
        kind: &str,
        data: &[u8],
        opts: &JobOptions,
    ) -> Result<Option<String>, QueueError> {
        if !crate::security::valid_idempotency_key(&job.idempotency_key) {
            return Err(QueueError::BadKey);
        }
        // IMMEDIATE, not deferred: the write lock is taken up front,
        // so a concurrent enqueue on another connection waits on the
        // busy timeout and then sees this job as a duplicate — a
        // deferred transaction would fail spuriously with SQLITE_BUSY
        // when upgrading its read lock mid-check.
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let is_duplicate: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs WHERE idempotency_key=?1)",
            params![job.idempotency_key],
            |r| r.get::<_, i64>(0),
        )? == 1;
        if is_duplicate {
            // Same key, different document: the first writer wins
            // (the stored job is NEVER overwritten), but the conflict
            // is logged — it means a client bug or a key being reused
            // across different prints. The comparison uses the stored
            // payload fingerprint (length + hash), never the whole
            // blob, and lookup errors propagate: a database that
            // cannot answer must not silently read as "no conflict"
            // (audit M3).
            let existing: Option<(String, i64)> = tx
                .query_row(
                    "SELECT id, bytes FROM jobs WHERE idempotency_key=?1",
                    params![job.idempotency_key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let payload_differs = match &existing {
                Some((existing_id, _)) => {
                    let stored: Option<(i64, Option<i64>)> = tx
                        .query_row(
                            "SELECT length(data), payload_hash FROM payloads WHERE job_id=?1",
                            params![existing_id],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    match stored {
                        // Rows written before the hash column existed
                        // compare by length only.
                        Some((len, hash)) => {
                            len as usize != data.len()
                                || hash.is_some_and(|h| h != payload_fingerprint(data))
                        }
                        None => false,
                    }
                }
                None => false,
            };
            drop(tx);
            if let Some((existing_id, existing_bytes)) = existing {
                if existing_bytes as u64 != job.bytes || payload_differs {
                    self.record_event_best_effort(
                        &existing_id,
                        "duplicate_conflict",
                        Some(&format!(
                            "existing_bytes={existing_bytes} submitted_bytes={}",
                            job.bytes
                        )),
                    );
                }
            }
            return Ok(None);
        }
        let active_printer: i64 = tx.query_row(
            "SELECT COUNT(*) FROM jobs WHERE printer=?1 AND state IN ('queued','rendering','spooling','printing')",
            params![job.printer],
            |r| r.get(0),
        )?;
        if active_printer as u64 >= self.max_active_per_printer {
            return Err(QueueError::QueueFull(job.printer.clone()));
        }
        let active_global: i64 = tx.query_row(
            "SELECT COUNT(*) FROM jobs WHERE state IN ('queued','rendering','spooling','printing')",
            [],
            |r| r.get(0),
        )?;
        if active_global as u64 >= self.max_pending_global {
            return Err(QueueError::QueueFull("global".to_string()));
        }
        tx.execute(
            "INSERT INTO jobs(id,idempotency_key,printer,report,pages,bytes,priority,state,attempts,error_code,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'queued',0,NULL,?8)",
            params![job.id, job.idempotency_key, job.printer, job.report, job.pages, job.bytes, job.priority, now_unix()],
        )?;
        tx.execute(
            "INSERT INTO payloads(job_id, kind, data, payload_hash) VALUES(?1,?2,?3,?4)",
            params![job.id, kind, data, payload_fingerprint(data)],
        )?;
        let json = serde_json::to_string(opts).unwrap_or_else(|_| "{}".to_string());
        tx.execute(
            "INSERT INTO job_options(job_id, options_json) VALUES(?1,?2)",
            params![job.id, json],
        )?;
        tx.execute(
            "INSERT INTO job_events(job_id, ts, event, detail) VALUES(?1,?2,'enqueued',?3)",
            params![job.id, now_unix(), diag_id_for_key(&job.idempotency_key)],
        )?;
        tx.commit()?;
        Ok(Some(job.id.clone()))
    }

    /// The job id stored for an idempotency key, if any.
    pub fn job_id_for_key(&self, key: &str) -> Result<Option<String>, QueueError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM jobs WHERE idempotency_key=?1")?;
        let mut rows = stmt.query(params![key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// Store a job's document payload (the queue row holds metadata; the
    /// bytes live here so the worker can print after any restart).
    /// kind: "pdf" | "escpos" | "zpl" | "tspl" | "image-png".
    pub fn store_payload(
        &mut self,
        job_id: &str,
        kind: &str,
        data: &[u8],
    ) -> Result<(), QueueError> {
        // The queue is the durable layer: enforce the size cap and
        // job existence here too, not only in the app crate, so an
        // oversize blob or an orphan payload (no job row — invisible
        // to every retention path) can never be persisted (audit L5).
        if data.len() > crate::security::MAX_JOB_BYTES {
            return Err(QueueError::PayloadTooLarge(crate::security::MAX_JOB_BYTES));
        }
        let job_exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1)",
            params![job_id],
            |r| r.get::<_, i64>(0),
        )? == 1;
        if !job_exists {
            return Err(QueueError::NotFound(job_id.to_string()));
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO payloads(job_id, kind, data, payload_hash) VALUES(?1,?2,?3,?4)",
            params![job_id, kind, data, payload_fingerprint(data)],
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

    /// Store execution options for a job.
    pub fn store_options(&mut self, job_id: &str, opts: &JobOptions) -> Result<(), QueueError> {
        let json = serde_json::to_string(opts).unwrap_or_else(|_| "{}".to_string());
        self.conn.execute(
            "INSERT OR REPLACE INTO job_options(job_id, options_json) VALUES(?1,?2)",
            params![job_id, json],
        )?;
        Ok(())
    }

    /// Read execution options for a job (falls back to `JobOptions::default()`).
    pub fn options(&self, job_id: &str) -> Result<JobOptions, QueueError> {
        let mut stmt = self
            .conn
            .prepare("SELECT options_json FROM job_options WHERE job_id=?1")?;
        let mut rows = stmt.query(params![job_id])?;
        match rows.next()? {
            Some(r) => {
                let raw: String = r.get(0)?;
                // A stored row that no longer parses is corruption,
                // not a default: silently substituting GDI defaults
                // would re-route the job to the wrong print path.
                serde_json::from_str(&raw).map_err(|e| QueueError::BadOptions(e.to_string()))
            }
            None => Ok(JobOptions::default()),
        }
    }

    /// Retention bound for terminal failure states: drop the stored
    /// document bytes of jobs that ended `failed`/`dead_letter` more
    /// than `max_age_secs` ago (by their latest event timestamp). The
    /// job rows and events stay at this stage, so recent history and
    /// diagnostics survive; only the (potentially large) payloads are
    /// reclaimed. Rows themselves are reclaimed later by
    /// `purge_stale_failed_jobs`. Returns how many payloads were
    /// dropped.
    pub fn purge_stale_failed_payloads(&mut self, max_age_secs: i64) -> Result<usize, QueueError> {
        let cutoff = now_unix().saturating_sub(max_age_secs);
        let n = self.conn.execute(
            "DELETE FROM payloads WHERE job_id IN (
               SELECT j.id FROM jobs j
               WHERE j.state IN ('failed','dead_letter')
                 AND (SELECT MAX(e.ts) FROM job_events e WHERE e.job_id = j.id) < ?1
             )",
            params![cutoff],
        )?;
        Ok(n)
    }

    /// Second retention stage: delete the `jobs` rows (with their
    /// events, options and any remaining payload) of
    /// `failed`/`dead_letter` jobs whose latest event is older than
    /// `max_age_secs`. Without this the history tables grow for the
    /// life of the install — the payload purge alone bounds bytes,
    /// not rows (audit M1). Callers pass a much longer window than
    /// the payload purge (the app uses 90 days vs 30). Returns how
    /// many job rows were deleted.
    pub fn purge_stale_failed_jobs(&mut self, max_age_secs: i64) -> Result<usize, QueueError> {
        let cutoff = now_unix().saturating_sub(max_age_secs);
        let ids: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT j.id FROM jobs j
                 WHERE j.state IN ('failed','dead_letter')
                   AND (SELECT MAX(e.ts) FROM job_events e WHERE e.job_id = j.id) < ?1",
            )?;
            let rows = stmt.query_map(params![cutoff], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            out
        };
        let mut n = 0usize;
        for id in &ids {
            self.conn
                .execute("DELETE FROM job_events WHERE job_id=?1", params![id])?;
            self.conn
                .execute("DELETE FROM job_options WHERE job_id=?1", params![id])?;
            self.conn
                .execute("DELETE FROM payloads WHERE job_id=?1", params![id])?;
            n += self
                .conn
                .execute("DELETE FROM jobs WHERE id=?1", params![id])?;
        }
        Ok(n)
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
        // Guarded write: the UPDATE only lands if the stored state is
        // still the state we validated against. Two connections (worker
        // + reconciler, app + probe tool) can no longer interleave a
        // read-check-update into a transition the machine forbids; the
        // loser sees 0 rows changed and gets the transition error.
        let changed = self.conn.execute(
            "UPDATE jobs SET state=?1, error_code=?2, updated_at=?3 WHERE id=?4 AND state=?5",
            params![
                next.as_str(),
                error_code,
                now_unix(),
                id,
                job.state.as_str()
            ],
        )?;
        if changed == 0 {
            return Err(QueueError::Transition(crate::state::TransitionError {
                from: job.state,
                to: next,
            }));
        }
        self.record_event_best_effort(id, &format!("state:{}", next.as_str()), error_code);
        Ok(())
    }

    /// Atomically claim the next queued job for a printer (queued -> rendering).
    /// The UPDATE ... WHERE state='queued' guard makes double-claim impossible
    /// even across concurrent connections.
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
        let updated = self.conn.execute(
            "UPDATE jobs SET state='rendering', error_code=NULL, attempts=attempts+1, updated_at=?2 WHERE id=?1 AND state='queued'",
            params![id, now_unix()],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        let job = self.get(&id)?;
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
            "UPDATE jobs SET state='queued', error_code='RECOVERED_BEFORE_SPOOL', updated_at=?1 WHERE state IN ('rendering','spooling')",
            params![now_unix()])? as u64;
        let (state, code) = if auto_reprint {
            ("queued", "RECOVERED_AUTO_REPRINT")
        } else {
            ("failed", "RECOVERY_AMBIGUOUS")
        };
        let ambiguous = self.conn.execute(
            "UPDATE jobs SET state=?1, error_code=?2, updated_at=?3 WHERE state='printing'",
            params![state, code, now_unix()],
        )? as u64;
        self.record_event_best_effort(
            "__queue__",
            "recovery",
            Some(&format!("requeued={requeued} ambiguous={ambiguous}")),
        );
        Ok((requeued, ambiguous))
    }

    /// Reconciliation for wedged jobs: a job sitting in a working
    /// state (rendering/spooling) whose last update is older
    /// than STALE_JOB_THRESHOLD_SECS belongs to a worker that is gone
    /// or stuck. Fail it loudly with RECONCILED_STALE — the user can
    /// retry it — instead of letting it block the queue forever.
    /// Jobs predating the updated_at column (updated_at = 0) are
    /// never reconciled: absence of evidence is not evidence of a
    /// wedge. Returns how many jobs were reconciled.
    pub fn reconcile_stale(&mut self) -> Result<u64, QueueError> {
        let cutoff = now_unix() - STALE_JOB_THRESHOLD_SECS;
        let ids: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT id FROM jobs WHERE state IN ('rendering','spooling') AND updated_at > 0 AND updated_at < ?1",
            )?;
            let rows = stmt.query_map(params![cutoff], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            out
        };
        let mut n = 0u64;
        for id in ids {
            // Direct terminal write: reconciliation must succeed even
            // for states the normal machine would route differently;
            // 'failed' is reachable from every working state anyway.
            let changed = self.conn.execute(
                "UPDATE jobs SET state='failed', error_code='RECONCILED_STALE', updated_at=?1 WHERE id=?2 AND state IN ('rendering','spooling')",
                params![now_unix(), id],
            )?;
            if changed > 0 {
                self.record_event_best_effort(&id, "reconciled_stale", Some("RECONCILED_STALE"));
                n += 1;
            }
        }
        Ok(n)
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
        // Clamp at read: a negative stored value (manual DB edit,
        // corruption, a future writer bug) must not wrap into a huge
        // unsigned and poison diagnostics/accounting (audit L4).
        pages: r.get::<_, i64>(4)?.clamp(0, i64::from(u32::MAX)) as u32,
        bytes: r.get::<_, i64>(5)?.max(0) as u64,
        priority: r.get(6)?,
        state: JobState::parse(&state_s).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unknown job state in database: {state_s}"),
                )),
            )
        })?,
        attempts: r.get::<_, i64>(8)?.clamp(0, i64::from(u32::MAX)) as u32,
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
    fn stale_failed_payloads_are_purged_but_rows_stay() {
        let mut q = Queue::in_memory().unwrap();
        let old = job("job-old-fail", "key-old", "P1", 0);
        q.enqueue_with_payload(&old, "pdf", b"%PDF-old", &JobOptions::default())
            .unwrap();
        q.set_state(&old.id, JobState::Failed, Some("BOOM"))
            .unwrap();
        // A far-future cutoff treats the just-failed job as stale.
        let dropped = q.purge_stale_failed_payloads(-31_557_600_000).unwrap();
        assert_eq!(dropped, 1);
        assert!(q.payload(&old.id).unwrap().is_none());
        assert_eq!(q.get(&old.id).unwrap().state, JobState::Failed);
        // A freshly failed job keeps its payload inside the window.
        let fresh = job("job-new-fail", "key-new", "P1", 0);
        q.enqueue_with_payload(&fresh, "pdf", b"%PDF-new", &JobOptions::default())
            .unwrap();
        q.set_state(&fresh.id, JobState::Failed, Some("BOOM"))
            .unwrap();
        assert_eq!(q.purge_stale_failed_payloads(86_400).unwrap(), 0);
        assert!(q.payload(&fresh.id).unwrap().is_some());
    }

    #[test]
    fn stale_failed_job_rows_are_purged_after_long_window() {
        let mut q = Queue::in_memory().unwrap();
        let old = job("job-old-row", "key-old-row", "P1", 0);
        q.enqueue_with_payload(&old, "pdf", b"%PDF-old", &JobOptions::default())
            .unwrap();
        q.set_state(&old.id, JobState::Failed, Some("BOOM"))
            .unwrap();
        // Inside the window the row (and its events) survive.
        assert_eq!(q.purge_stale_failed_jobs(86_400).unwrap(), 0);
        assert!(q.get(&old.id).is_ok());
        // Past the window the row, its events and its payload all go.
        assert_eq!(q.purge_stale_failed_jobs(-31_557_600_000).unwrap(), 1);
        assert!(q.get(&old.id).is_err());
        assert!(q.payload(&old.id).unwrap().is_none());
        assert!(q.recent_events(100).unwrap().is_empty());
        // Done jobs are never touched by the failure retention.
        let done = job("job-done-row", "key-done-row", "P1", 0);
        q.enqueue_with_payload(&done, "pdf", b"%PDF-done", &JobOptions::default())
            .unwrap();
        for s in [
            JobState::Rendering,
            JobState::Spooling,
            JobState::Printing,
            JobState::Done,
        ] {
            q.set_state(&done.id, s, None).unwrap();
        }
        assert_eq!(q.purge_stale_failed_jobs(-31_557_600_000).unwrap(), 0);
        assert!(q.get(&done.id).is_ok());
    }

    #[test]
    fn store_payload_rejects_unknown_job_and_oversize() {
        let mut q = Queue::in_memory().unwrap();
        assert!(matches!(
            q.store_payload("ghost", "pdf", b"%PDF"),
            Err(QueueError::NotFound(_))
        ));
        q.enqueue(&job("1", "k-sp", "p1", 0)).unwrap();
        let big = vec![0u8; crate::security::MAX_JOB_BYTES + 1];
        assert!(matches!(
            q.store_payload("1", "pdf", &big),
            Err(QueueError::PayloadTooLarge(_))
        ));
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
    fn duplicate_submission_never_overwrites_payload() {
        let mut q = Queue::in_memory().unwrap();
        let first = q.enqueue_new(&job("1", "k-dup", "p1", 0)).unwrap();
        assert_eq!(first.as_deref(), Some("1"));
        q.store_payload("1", "pdf", b"ORIGINAL").unwrap();
        // Same key, different job id and different bytes: not new, and
        // the stored payload stays the original.
        let dup = q.enqueue_new(&job("2", "k-dup", "p1", 0)).unwrap();
        assert_eq!(dup, None);
        assert_eq!(q.job_id_for_key("k-dup").unwrap().as_deref(), Some("1"));
        let (kind, data) = q.payload("1").unwrap().unwrap();
        assert_eq!(kind, "pdf");
        assert_eq!(data, b"ORIGINAL");
    }
    #[test]
    fn priority_then_fifo_claim() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&job("bulk", "k-bulk", "p1", 0)).unwrap();
        q.enqueue(&job("receipt", "k-rcpt", "p1", 10)).unwrap();
        let first = q.claim_next("p1").unwrap().unwrap();
        assert_eq!(first.id, "receipt");
        assert_eq!(first.state, JobState::Rendering);
        assert_eq!(first.attempts, 1);
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
    fn atomic_enqueue_writes_job_payload_options_together() {
        let mut q = Queue::in_memory().unwrap();
        let opts = JobOptions {
            mode: "escpos".into(),
            ..JobOptions::default()
        };
        let id = q
            .enqueue_with_payload(
                &job("a1", "k-atomic", "p1", 0),
                "image-png",
                b"PNGDATA",
                &opts,
            )
            .unwrap();
        assert_eq!(id.as_deref(), Some("a1"));
        let (kind, data) = q.payload("a1").unwrap().unwrap();
        assert_eq!(kind, "image-png");
        assert_eq!(data, b"PNGDATA");
        assert_eq!(q.options("a1").unwrap().mode, "escpos");
        // Duplicate: nothing written, original payload untouched.
        let dup = q
            .enqueue_with_payload(
                &job("a2", "k-atomic", "p1", 0),
                "image-png",
                b"OTHER",
                &opts,
            )
            .unwrap();
        assert_eq!(dup, None);
        assert_eq!(q.count_by_state(JobState::Queued).unwrap(), 1);
        let (_, data) = q.payload("a1").unwrap().unwrap();
        assert_eq!(data, b"PNGDATA");
        // The enqueue was logged as a structured event with the diag id.
        let events = q.recent_events(10).unwrap();
        assert!(events.iter().any(|e| e.event == "enqueued"
            && e.detail.as_deref() == Some(diag_id_for_key("k-atomic").as_str())));
    }

    #[test]
    fn concurrent_same_key_enqueues_create_exactly_one_job() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("np-conc-{}.db", std::process::id()));
        let p = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);
        // Create the schema up front so the two connections race only
        // on the enqueue itself.
        drop(Queue::open(&p).unwrap());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for who in ["t1", "t2"] {
            let (pp, bb) = (p.clone(), barrier.clone());
            handles.push(std::thread::spawn(move || {
                let mut q = Queue::open(&pp).unwrap();
                let j = Job {
                    id: who.into(),
                    idempotency_key: "k-race".into(),
                    printer: "p1".into(),
                    report: None,
                    pages: 1,
                    bytes: 10,
                    priority: 0,
                    state: JobState::Queued,
                    attempts: 0,
                    error_code: None,
                };
                bb.wait();
                q.enqueue_with_payload(&j, "pdf", b"X", &JobOptions::default())
                    .unwrap()
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let news = results.iter().filter(|r| r.is_some()).count();
        assert_eq!(
            news, 1,
            "exactly one thread may win the enqueue: {results:?}"
        );
        let q = Queue::open(&p).unwrap();
        assert_eq!(q.count_by_state(JobState::Queued).unwrap(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn duplicate_with_different_payload_is_logged_as_conflict() {
        let mut q = Queue::in_memory().unwrap();
        let opts = JobOptions::default();
        let mut first = job("c1", "k-conflict", "p1", 0);
        first.bytes = 100;
        q.enqueue_with_payload(&first, "pdf", b"0123456789", &opts)
            .unwrap();
        // Same key, same size: an ordinary duplicate — no conflict.
        let mut same = job("c2", "k-conflict", "p1", 0);
        same.bytes = 100;
        assert_eq!(
            q.enqueue_with_payload(&same, "pdf", b"0123456789", &opts)
                .unwrap(),
            None
        );
        assert!(q
            .recent_events(10)
            .unwrap()
            .iter()
            .all(|e| e.event != "duplicate_conflict"));
        // Same key, DIFFERENT size: first writer still wins, and the
        // conflict lands in the structured event log.
        let mut other = job("c3", "k-conflict", "p1", 0);
        other.bytes = 999;
        assert_eq!(
            q.enqueue_with_payload(&other, "pdf", b"zzz", &opts)
                .unwrap(),
            None
        );
        let events = q.recent_events(10).unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.event == "duplicate_conflict" && e.job_id == "c1"),
            "{events:?}"
        );
        assert_eq!(q.get("c1").unwrap().bytes, 100);
    }

    #[test]
    fn global_limit_rejects_beyond_cap() {
        let mut q = Queue::in_memory().unwrap();
        q.set_max_pending_global(1);
        q.enqueue(&job("g1", "k-g1", "printer-a", 0)).unwrap();
        // A different printer does not help: the cap is global.
        let err = q.enqueue(&job("g2", "k-g2", "printer-b", 0)).unwrap_err();
        assert!(matches!(err, QueueError::QueueFull(_)), "{err}");
        // Duplicates of an existing key are not new load.
        assert!(q.enqueue(&job("g1b", "k-g1", "printer-a", 0)).is_ok());
    }

    #[test]
    fn retry_keeps_attempts_monotonic() {
        let mut q = Queue::in_memory().unwrap();
        let j = job("j1", "k1", "P1", 0);
        q.enqueue_with_payload(&j, "pdf", b"data", &JobOptions::default())
            .unwrap();
        let claimed = q.claim_next("P1").unwrap().unwrap();
        assert_eq!(claimed.attempts, 1);
        q.set_state("j1", JobState::Failed, Some("E")).unwrap();
        q.retry_job("j1").unwrap();
        let after = q.get("j1").unwrap();
        assert_eq!(after.state, JobState::Queued);
        assert_eq!(after.attempts, 1, "attempts must never decrease");
    }

    #[test]
    fn reconcile_never_fails_a_printing_job() {
        let mut q = Queue::in_memory().unwrap();
        let j = job("j2", "k2", "P1", 0);
        q.enqueue_with_payload(&j, "pdf", b"data", &JobOptions::default())
            .unwrap();
        q.claim_next("P1").unwrap();
        q.set_state("j2", JobState::Spooling, None).unwrap();
        q.set_state("j2", JobState::Printing, None).unwrap();
        q.conn
            .execute(
                "UPDATE jobs SET updated_at=?1 WHERE id='j2'",
                params![now_unix() - STALE_JOB_THRESHOLD_SECS - 60],
            )
            .unwrap();
        // A long physical print is not a wedge: reconciliation must
        // leave 'printing' to startup recovery (ambiguous-outcome path).
        assert_eq!(q.reconcile_stale().unwrap(), 0);
        assert_eq!(q.get("j2").unwrap().state, JobState::Printing);
    }

    #[test]
    fn reconcile_stale_fails_only_old_working_jobs() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&job("s1", "k-s1", "p1", 0)).unwrap();
        let claimed = q.claim_next("p1").unwrap().unwrap();
        assert_eq!(claimed.state, JobState::Rendering);
        // Fresh working job: untouched.
        assert_eq!(q.reconcile_stale().unwrap(), 0);
        // Age it beyond the threshold: reconciled to failed.
        q.conn
            .execute(
                "UPDATE jobs SET updated_at=?1 WHERE id='s1'",
                params![now_unix() - STALE_JOB_THRESHOLD_SECS - 10],
            )
            .unwrap();
        assert_eq!(q.reconcile_stale().unwrap(), 1);
        let after = q.get("s1").unwrap();
        assert_eq!(after.state, JobState::Failed);
        assert_eq!(after.error_code.as_deref(), Some("RECONCILED_STALE"));
        // Queued jobs are never reconciled, however old.
        q.enqueue(&job("s2", "k-s2", "p1", 0)).unwrap();
        q.conn
            .execute("UPDATE jobs SET updated_at=1 WHERE id='s2'", [])
            .unwrap();
        assert_eq!(q.reconcile_stale().unwrap(), 0);
        assert_eq!(q.get("s2").unwrap().state, JobState::Queued);
    }

    #[test]
    fn diag_id_is_stable_and_key_specific() {
        assert_eq!(diag_id_for_key("order-42"), diag_id_for_key("order-42"));
        assert_ne!(diag_id_for_key("order-42"), diag_id_for_key("order-43"));
        assert!(diag_id_for_key("order-42").starts_with("np-"));
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
    fn payload_and_options_round_trip_and_purge() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&job("1", "k-1", "p1", 0)).unwrap();
        assert!(q.payload("1").unwrap().is_none());
        q.store_payload("1", "pdf", b"%PDF-data").unwrap();
        q.store_options(
            "1",
            &JobOptions {
                mode: "escpos".into(),
                paper: Some("80mm".into()),
                copies: 2,
                ..Default::default()
            },
        )
        .unwrap();
        let (kind, data) = q.payload("1").unwrap().unwrap();
        assert_eq!(kind, "pdf");
        assert_eq!(data, b"%PDF-data");
        let opts = q.options("1").unwrap();
        assert_eq!(opts.mode, "escpos");
        assert_eq!(opts.copies, 2);
        q.purge_payload("1").unwrap();
        assert!(q.payload("1").unwrap().is_none());
    }
    #[test]
    fn list_retry_and_clear_finished() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&job("1", "k-1", "p1", 0)).unwrap();
        q.enqueue(&job("2", "k-2", "p2", 10)).unwrap();
        q.store_payload("2", "pdf", b"%PDF-cancel").unwrap();
        assert_eq!(q.queued_printers().unwrap(), vec!["p2", "p1"]);
        assert_eq!(q.list_recent(10).unwrap().len(), 2);
        q.set_state("1", JobState::Failed, Some("ERR")).unwrap();
        q.retry_job("1").unwrap();
        assert_eq!(q.get("1").unwrap().state, JobState::Queued);
        q.cancel_job("2").unwrap();
        assert!(q.payload("2").unwrap().is_none());
        assert_eq!(q.clear_finished().unwrap(), 1);
        assert_eq!(q.list_recent(10).unwrap().len(), 1);
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
