use crate::queue::{Job, Queue, QueueError};
use serde::Serialize;

/// One-click diagnostics bundle content (spec section 8): versions, queue
/// statistics, and recent jobs — with job content and user identity
/// redacted by construction (this schema has no field that could hold them).
#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsBundle {
    pub app_version: String,
    pub core_version: String,
    pub os: String,
    pub state_counts: Vec<(String, u64)>,
    pub recent_jobs: Vec<JobSummary>,
    /// Structured job events (the durable diagnostics log): state
    /// transitions, recoveries, reconciliations — metadata only,
    /// never document content.
    #[serde(default)]
    pub recent_events: Vec<crate::queue::JobEvent>,
    /// Set when the events query itself failed: support must see
    /// "events unreadable" instead of a misleading empty list
    /// (audit L13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobSummary {
    pub id: String,
    /// Diagnostics correlation id (stable fingerprint of the job's
    /// idempotency key): quote it in support reports.
    pub diag_id: String,
    pub printer: String,
    pub report: Option<String>,
    pub pages: u32,
    pub bytes: u64,
    pub state: String,
    pub attempts: u32,
    pub error_code: Option<String>,
}

/// Strip `user[:pass]@` credentials from any `scheme://userinfo@host` URI
/// so diagnostics bundles never leak embedded network printer credentials.
fn redact_uri_credentials(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(scheme_end) = rest.find("://") {
        let after_scheme = scheme_end + 3;
        out.push_str(&rest[..after_scheme]);
        rest = &rest[after_scheme..];
        let authority_end = rest
            .find(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace())
            .unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        if let Some(at) = authority.rfind('@') {
            out.push_str("[redacted]");
            out.push_str(&authority[at..]);
        } else {
            out.push_str(authority);
        }
        rest = &rest[authority_end..];
    }
    out.push_str(rest);
    out
}

impl From<&Job> for JobSummary {
    fn from(j: &Job) -> Self {
        Self {
            id: j.id.clone(),
            diag_id: crate::queue::diag_id_for_key(&j.idempotency_key),
            printer: redact_uri_credentials(&j.printer),
            report: j.report.clone(),
            pages: j.pages,
            bytes: j.bytes,
            state: j.state.as_str().to_string(),
            attempts: j.attempts,
            error_code: j.error_code.as_deref().map(redact_uri_credentials),
        }
    }
}

/// Build the bundle; at most the last 200 job summaries are included.
pub fn build_bundle(queue: &Queue, recent: &[JobSummary]) -> Result<DiagnosticsBundle, QueueError> {
    let counts: Vec<(String, u64)> = queue
        .count_all_states()?
        .into_iter()
        .map(|(s, n)| (s.as_str().to_string(), n))
        .collect();
    let (recent_events, events_error) = match queue.recent_events(100) {
        Ok(events) => (
            events
                .into_iter()
                .map(|mut ev| {
                    ev.detail = ev.detail.as_deref().map(redact_uri_credentials);
                    ev
                })
                .collect(),
            None,
        ),
        Err(e) => (Vec::new(), Some(redact_uri_credentials(&e.to_string()))),
    };
    let recent_jobs = recent
        .iter()
        .take(200)
        .cloned()
        .map(|mut job| {
            job.printer = redact_uri_credentials(&job.printer);
            job.error_code = job.error_code.as_deref().map(redact_uri_credentials);
            job
        })
        .collect();
    Ok(DiagnosticsBundle {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        core_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        state_counts: counts,
        recent_jobs,
        recent_events,
        events_error,
    })
}

/// Build the diagnostics bundle directly from the queue's recent jobs.
pub fn build_bundle_from_queue(queue: &Queue) -> Result<DiagnosticsBundle, QueueError> {
    let jobs = queue.list_recent(200)?;
    let summaries: Vec<JobSummary> = jobs.iter().map(JobSummary::from).collect();
    build_bundle(queue, &summaries)
}

pub fn bundle_to_json(b: &DiagnosticsBundle) -> String {
    serde_json::to_string_pretty(b).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::Job;
    use crate::state::JobState;
    #[test]
    fn bundle_counts_and_redaction() {
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&Job {
            id: "1".into(),
            idempotency_key: "k-1".into(),
            printer: "p1".into(),
            report: Some("sale.order".into()),
            pages: 2,
            bytes: 99,
            priority: 0,
            state: JobState::Queued,
            attempts: 0,
            error_code: None,
        })
        .unwrap();
        let b = build_bundle_from_queue(&q).unwrap();
        assert_eq!(b.recent_jobs.len(), 1);
        let json = bundle_to_json(&b);
        assert!(json.contains("\"queued\""));
        // The bundle schema has no field for document content or secrets.
        assert!(!json.contains("session_id"));
        assert!(!json.contains("password"));
    }

    #[test]
    fn uri_credentials_are_redacted_in_diagnostics() {
        assert_eq!(
            redact_uri_credentials("ipp://admin:s3cr3t@192.168.1.50:631/ipp/print"),
            "ipp://[redacted]@192.168.1.50:631/ipp/print"
        );
        assert_eq!(redact_uri_credentials(r"\\POS-1\XP-80C"), r"\\POS-1\XP-80C");
        assert_eq!(redact_uri_credentials("HP LaserJet Pro"), "HP LaserJet Pro");
        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&Job {
            id: "u1".into(),
            idempotency_key: "k-u1".into(),
            printer: "ipp://admin:s3cr3t@192.168.1.50:631/ipp/print".into(),
            report: None,
            pages: 1,
            bytes: 10,
            priority: 0,
            state: JobState::Queued,
            attempts: 0,
            error_code: None,
        })
        .unwrap();
        q.set_state(
            "u1",
            JobState::Failed,
            Some("http://user:hunter2@10.0.0.5:631/ipp"),
        )
        .unwrap();
        let json = bundle_to_json(&build_bundle_from_queue(&q).unwrap());
        assert!(!json.contains("s3cr3t"));
        assert!(!json.contains("hunter2"));
        assert!(json.contains("[redacted]@192.168.1.50:631/ipp/print"));
        assert!(json.contains("[redacted]@10.0.0.5:631/ipp"));
    }
}
