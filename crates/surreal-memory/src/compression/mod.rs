//! Evidence-backed compression of memories (issue #54).
//!
//! Pipeline: a [`Summarizer`] proposes quoted claims, the [`gate`] validates
//! them against the exact source text and token budget, [`promotion`] decides
//! which may act as guidance, and storage publishes the result atomically.
//! Stored content is evidence to evaluate, never an instruction.

pub mod clock;
pub mod contract;
pub mod gate;
pub mod lineage;
pub mod promotion;
pub mod provider;
pub mod summarizer;
pub mod text;

pub use clock::{Clock, FixedClock, SystemClock};
pub use contract::{
    Applicability, CONTRACT_VERSION, Candidate, ClaimKind, ClaimRecord, ClaimStatus,
    EvidenceRecord, EvidenceRef, Qualifiers, SourceMemory, SourceOrigin, SummarizerDescriptor,
    SummaryMethod,
};
pub use gate::{DeferReason, GatedSummary, Limits, RejectReason, Rejected, ValidatedClaim};
pub use promotion::{EvidenceStrength, Promotion};
pub use summarizer::{ExtractiveSummarizer, SummarizeError, Summarizer};

use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Points where a test can make a derivation fail, to prove a failure at any
/// stage leaves sources, claims and counters unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultPoint {
    Generation,
    Validation,
    Insertion,
    Activation,
}

/// Everything a storage needs to run the pipeline. The default is the
/// no-network extractive summarizer, so nothing leaves the process unless a
/// provider is configured explicitly.
#[derive(Clone)]
pub struct CompressionRuntime {
    pub summarizer: Arc<dyn Summarizer>,
    pub limits: Limits,
    pub clock: Arc<dyn Clock>,
    pub summarizer_timeout: Duration,
    faults: Arc<Mutex<Vec<FaultPoint>>>,
}

impl Default for CompressionRuntime {
    fn default() -> Self {
        Self {
            summarizer: Arc::new(ExtractiveSummarizer),
            limits: Limits::default(),
            clock: Arc::new(SystemClock),
            summarizer_timeout: Duration::from_secs(30),
            faults: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl CompressionRuntime {
    pub fn with_summarizer(mut self, summarizer: Arc<dyn Summarizer>) -> Self {
        self.summarizer = summarizer;
        self
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_summarizer_timeout(mut self, timeout: Duration) -> Self {
        self.summarizer_timeout = timeout;
        self
    }

    /// Test hook: the next pass through `point` fails once.
    #[doc(hidden)]
    pub fn inject_fault(&self, point: FaultPoint) {
        if let Ok(mut faults) = self.faults.lock() {
            faults.push(point);
        }
    }

    /// True (and consumed) when a fault was injected at `point`.
    pub(crate) fn fault(&self, point: FaultPoint) -> bool {
        let Ok(mut faults) = self.faults.lock() else {
            return false;
        };
        match faults.iter().position(|p| *p == point) {
            Some(index) => {
                faults.remove(index);
                true
            }
            None => false,
        }
    }
}
