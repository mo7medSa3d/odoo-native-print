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

fn is_secret_delim(b: u8) -> bool {
    b.is_ascii_whitespace() || matches!(b, b'&' | b';' | b',' | b'"' | b'\'' | b')' | b']' | b'}')
}

/// Redact URI credentials, Itbaa license keys (`ITBAA1.*`), `Bearer` tokens,
/// and common secret `key=value` fields so diagnostics never leak secrets.
fn redact_secrets(input: &str) -> String {
    let step1 = redact_uri_credentials(input);
    // Redact Itbaa license keys (`ITBAA1.<payload>.<sig>`).
    let mut step2 = String::with_capacity(step1.len());
    let mut i = 0;
    let bytes = step1.as_bytes();
    while i < bytes.len() {
        if i + 7 <= bytes.len() && bytes[i..i + 7].eq_ignore_ascii_case(b"ITBAA1.") {
            step2.push_str("[redacted-license]");
            i += 7;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || matches!(bytes[i], b'-' | b'_' | b'.' | b'='))
            {
                i += 1;
            }
        } else {
            let ch = step1[i..].chars().next().unwrap_or(' ');
            step2.push(ch);
            i += ch.len_utf8();
        }
    }
    // Redact `Bearer <token>` / `Basic <token>` and sensitive `key=value` / `key: value` pairs.
    const SECRET_KEYS: [&str; 9] = [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "session_id",
        "license_key",
        "authorization",
    ];
    let mut out = String::with_capacity(step2.len());
    let b2 = step2.as_bytes();
    let mut j = 0;
    while j < b2.len() {
        let at_boundary = j == 0
            || b2[j - 1].is_ascii_whitespace()
            || matches!(
                b2[j - 1],
                b'?' | b'&' | b';' | b',' | b'"' | b'\'' | b'(' | b'[' | b'{'
            );
        if at_boundary && j + 7 <= b2.len() && b2[j..j + 7].eq_ignore_ascii_case(b"bearer ") {
            out.push_str("Bearer ");
            j += 7;
            while j < b2.len() && b2[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < b2.len() && !is_secret_delim(b2[j]) {
                out.push_str("[redacted]");
                while j < b2.len() && !is_secret_delim(b2[j]) {
                    j += 1;
                }
            }
            continue;
        }
        if at_boundary {
            if let Some(&key) = SECRET_KEYS.iter().find(|&&k| {
                j + k.len() < b2.len()
                    && b2[j..j + k.len()].eq_ignore_ascii_case(k.as_bytes())
                    && matches!(b2[j + k.len()], b'=' | b':')
            }) {
                let sep_idx = j + key.len();
                out.push_str(&step2[j..=sep_idx]);
                j = sep_idx + 1;
                while j < b2.len() && b2[j].is_ascii_whitespace() {
                    out.push(b2[j] as char);
                    j += 1;
                }
                if key == "authorization" {
                    for scheme in [b"bearer ".as_slice(), b"basic ".as_slice()] {
                        if j + scheme.len() <= b2.len()
                            && b2[j..j + scheme.len()].eq_ignore_ascii_case(scheme)
                        {
                            out.push_str(&step2[j..j + scheme.len()]);
                            j += scheme.len();
                            while j < b2.len() && b2[j].is_ascii_whitespace() {
                                j += 1;
                            }
                            break;
                        }
                    }
                }
                if j < b2.len() && matches!(b2[j], b'"' | b'\'') {
                    let quote = b2[j];
                    out.push(quote as char);
                    j += 1;
                    out.push_str("[redacted]");
                    while j < b2.len() && b2[j] != quote {
                        j += 1;
                    }
                    if j < b2.len() {
                        out.push(quote as char);
                        j += 1;
                    }
                } else if j < b2.len() && !is_secret_delim(b2[j]) {
                    out.push_str("[redacted]");
                    while j < b2.len() && !is_secret_delim(b2[j]) {
                        j += 1;
                    }
                }
                continue;
            }
        }
        let ch = step2[j..].chars().next().unwrap_or(' ');
        out.push(ch);
        j += ch.len_utf8();
    }
    out
}

impl From<&Job> for JobSummary {
    fn from(j: &Job) -> Self {
        Self {
            id: j.id.clone(),
            diag_id: crate::queue::diag_id_for_key(&j.idempotency_key),
            printer: redact_secrets(&j.printer),
            report: j.report.as_deref().map(redact_secrets),
            pages: j.pages,
            bytes: j.bytes,
            state: j.state.as_str().to_string(),
            attempts: j.attempts,
            error_code: j.error_code.as_deref().map(redact_secrets),
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
                    ev.detail = ev.detail.as_deref().map(redact_secrets);
                    ev
                })
                .collect(),
            None,
        ),
        Err(e) => (Vec::new(), Some(redact_secrets(&e.to_string()))),
    };
    let recent_jobs = recent
        .iter()
        .take(200)
        .cloned()
        .map(|mut job| {
            job.printer = redact_secrets(&job.printer);
            job.report = job.report.as_deref().map(redact_secrets);
            job.error_code = job.error_code.as_deref().map(redact_secrets);
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

    #[test]
    fn license_keys_and_tokens_are_redacted_in_diagnostics() {
        let fake_key = "ITBAA1.eyJjIjoiQWNtZSIsIm8iOiJodHRwczovL2VycC5hY21lLmNvbSIsImV4cCI6MCwidCI6InBybyJ9.AAAAbbbbCCCC111122223333";
        assert_eq!(
            redact_secrets(&format!("rejected {fake_key} for ERR_LICENSE_REQUIRED")),
            "rejected [redacted-license] for ERR_LICENSE_REQUIRED"
        );
        assert_eq!(
            redact_secrets("Authorization: Bearer eyJhbGciOi.secret.jwt"),
            "Authorization: Bearer [redacted]"
        );
        assert_eq!(
            redact_secrets("طابعة اطبع ITBAA1.eyJjIjoiQWNtZSJ9.AAAA1111"),
            "طابعة اطبع [redacted-license]"
        );
        assert_eq!(
            redact_secrets("Bearer my-secret-token failed"),
            "Bearer [redacted] failed"
        );
        assert_eq!(
            redact_secrets("https://erp.acme.com/web?session_id=sess_999&db=prod"),
            "https://erp.acme.com/web?session_id=[redacted]&db=prod"
        );
        assert_eq!(
            redact_secrets("requeued=1 ambiguous=0 ERR_LICENSE_EXPIRED"),
            "requeued=1 ambiguous=0 ERR_LICENSE_EXPIRED"
        );

        let mut q = Queue::in_memory().unwrap();
        q.enqueue(&Job {
            id: "l1".into(),
            idempotency_key: "k-l1".into(),
            printer: "HP LaserJet".into(),
            report: Some("sale.order?token=tok_secret".into()),
            pages: 1,
            bytes: 10,
            priority: 0,
            state: JobState::Queued,
            attempts: 0,
            error_code: None,
        })
        .unwrap();
        q.set_state(
            "l1",
            JobState::Failed,
            Some(&format!("license {fake_key} password=topsecret")),
        )
        .unwrap();
        let json = bundle_to_json(&build_bundle_from_queue(&q).unwrap());
        assert!(!json.contains("eyJjIjoiQWNtZS"));
        assert!(!json.contains("AAAAbbbbCCCC"));
        assert!(!json.contains("topsecret"));
        assert!(!json.contains("tok_secret"));
        assert!(json.contains("[redacted-license]"));
        assert!(json.contains("password=[redacted]"));
        assert!(json.contains("token=[redacted]"));
    }
}
