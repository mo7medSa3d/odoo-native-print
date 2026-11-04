//! print-core: platform-independent print job core.
//! Queue, state machine, page model, paper ladder, profiles, security validation.
//! No platform (Win32) code lives here, so this crate is fully testable anywhere.

pub mod diagnostics;
pub mod page;
pub mod paper;
pub mod printer;
pub mod queue;
pub mod routing;
pub mod security;
pub mod state;
pub mod worker;

pub use page::{PageSize, Rotation};
pub use paper::{match_paper, Form, MatchKind, PaperError, ScalePolicy};
pub use printer::{PrinterCapabilities, PrinterInfo, PrinterKind, PrinterStatus};
pub use queue::{Job, JobOptions, Queue, QueueError};
pub use routing::{printer_pattern_matches, resolve_printer};
pub use state::{JobState, TransitionError};
pub use worker::{BackendError, PrintBackend, Worker, WorkerConfig};
