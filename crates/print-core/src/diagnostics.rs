use crate::queue::{Job, Queue, QueueError};
use crate::state::JobState;
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
}

#[derive(Debug, Clone, Serialize)]
pub struct JobSummary {
    pub id: String,
    pub printer: String,
    pub report: Option<String>,
    pub pages: u32,
    pub bytes: u64,
    pub state: String,
    pub attempts: u32,
    pub error_code: Option<String>,
}

impl From<&Job> for JobSummary {
    fn from(j: &Job) -> Self {
        Self {
            id: j.id.clone(),
            printer: j.printer.clone(),
            report: j.report.clone(),
            pages: j.pages,
            bytes: j.bytes,
            state: j.state.as_str().to_string(),
            attempts: j.attempts,
            error_code: j.error_code.clone(),
        }
    }
}

const ALL_STATES: [JobState; 8] = [
    JobState::Queued,
    JobState::Rendering,
    JobState::Spooling,
    JobState::Printing,
    JobState::Done,
    JobState::Failed,
    JobState::Canceled,
    JobState::DeadLetter,
];

/// Build the bundle; at most the last 200 job summaries are included.
pub fn build_bundle(
    queue: &Queue,
    recent: &[JobSummary],
) -> Result<DiagnosticsBundle, QueueError> {
    let mut counts = Vec::new();
    for s in ALL_STATES {
        counts.push((s.as_str().to_string(), queue.count_by_state(s)?));
    }
    Ok(DiagnosticsBundle {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        core_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        state_counts: counts,
        recent_jobs: recent.iter().take(200).cloned().collect(),
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
}
