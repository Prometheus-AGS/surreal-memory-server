//! Injectable time source so freshness rules can be tested with a fixed clock.
//!
//! The rest of the library reads the system clock directly; only the
//! compression code goes through this trait.

use chrono::{DateTime, Utc};

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock that always reports the same instant.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}
