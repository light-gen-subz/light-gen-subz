use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::Result;

use crate::errors::coded;

static CANCELLED: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// Held for the length of a transcription or a translation, so that quitting the app can
/// wait for it to clean up (temporary audio, ffmpeg) instead of cutting it short.
pub struct Busy(());

impl Busy {
    pub fn enter() -> Busy {
        RUNNING.fetch_add(1, Ordering::SeqCst);
        Busy(())
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn is_busy() -> bool {
    RUNNING.load(Ordering::SeqCst) > 0
}

/// Called when a run starts: a stale cancellation must not abort the next one.
pub fn reset() {
    CANCELLED.store(false, Ordering::SeqCst);
}

pub fn request() {
    CANCELLED.store(true, Ordering::SeqCst);
}

pub fn is_cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

/// `Err` once the user asked to stop; engines call it between units of work.
pub fn check() -> Result<()> {
    if is_cancelled() {
        Err(coded("cancelled", "cancelled by the user"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_guard_is_counted_until_it_is_dropped() {
        let guard = Busy::enter();
        assert!(is_busy());

        drop(guard);

        // Other tests may hold a guard at the same time: only the count matters here.
        let again = Busy::enter();
        assert!(is_busy());
        drop(again);
    }

    // One test: the flag is process-wide, so separate tests would race.
    #[test]
    fn a_request_is_seen_until_the_next_reset() {
        reset();
        assert!(check().is_ok());

        request();
        let err = check().unwrap_err();
        assert!(is_cancelled());
        assert!(err.to_string().contains("cancelled"));

        reset();
        assert!(check().is_ok());
    }
}
