use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Rendering,
    Spooling,
    Printing,
    Done,
    Failed,
    Canceled,
    DeadLetter,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Rendering => "rendering",
            JobState::Spooling => "spooling",
            JobState::Printing => "printing",
            JobState::Done => "done",
            JobState::Failed => "failed",
            JobState::Canceled => "canceled",
            JobState::DeadLetter => "dead_letter",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(JobState::Queued),
            "rendering" => Some(JobState::Rendering),
            "spooling" => Some(JobState::Spooling),
            "printing" => Some(JobState::Printing),
            "done" => Some(JobState::Done),
            "failed" => Some(JobState::Failed),
            "canceled" => Some(JobState::Canceled),
            "dead_letter" => Some(JobState::DeadLetter),
            _ => None,
        }
    }
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobState::Done | JobState::Canceled | JobState::DeadLetter
        )
    }
    /// The strict state machine from the spec (section 5, Job system).
    pub fn can_transition_to(&self, next: JobState) -> bool {
        use JobState::*;
        matches!(
            (self, next),
            (Queued, Rendering) | (Queued, Canceled) | (Queued, Failed)
                | (Rendering, Spooling) | (Rendering, Failed) | (Rendering, Canceled)
                | (Spooling, Printing) | (Spooling, Failed) | (Spooling, Canceled)
                | (Printing, Done) | (Printing, Failed)
                | (Failed, Queued)   // retry
                | (Failed, DeadLetter)
                | (DeadLetter, Queued) // one-click retry from dead letter
        )
    }
    pub fn transition_to(&self, next: JobState) -> Result<JobState, TransitionError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(TransitionError {
                from: *self,
                to: next,
            })
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid job state transition: {from:?} -> {to:?}")]
pub struct TransitionError {
    pub from: JobState,
    pub to: JobState,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn happy_path() {
        let mut s = JobState::Queued;
        for n in [
            JobState::Rendering,
            JobState::Spooling,
            JobState::Printing,
            JobState::Done,
        ] {
            s = s.transition_to(n).unwrap();
        }
        assert!(s.is_terminal());
    }
    #[test]
    fn invalid_jumps_rejected() {
        assert!(JobState::Queued.transition_to(JobState::Done).is_err());
        assert!(JobState::Done.transition_to(JobState::Queued).is_err());
        assert!(JobState::Printing.transition_to(JobState::Queued).is_err());
    }
    #[test]
    fn retry_paths() {
        assert!(JobState::Failed.transition_to(JobState::Queued).is_ok());
        assert!(JobState::Failed.transition_to(JobState::DeadLetter).is_ok());
        assert!(JobState::DeadLetter.transition_to(JobState::Queued).is_ok());
    }
}
