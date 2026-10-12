//! `NULL_TRACE=1`: how long starting and opening a file take, step by step, on stderr
//! (`null trace  412.3 ms  window open`). Nothing is measured otherwise.

use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static ON: OnceLock<bool> = OnceLock::new();

/// The clock starts (as early in `main` as it can).
pub fn start() {
    START.get_or_init(Instant::now);
}

pub fn on() -> bool {
    *ON.get_or_init(|| std::env::var_os("NULL_TRACE").is_some_and(|v| !v.is_empty() && v != "0"))
}

/// Says `what` happened, and when since the start.
pub fn mark(what: &str) {
    if on() {
        let since = START.get_or_init(Instant::now).elapsed();
        eprintln!("null trace {:>9.1} ms  {what}", since.as_secs_f64() * 1000.);
    }
}

/// A step timed: dropped, it says how long it took (and when it ended).
pub struct Step {
    what: String,
    at: Instant,
}

pub fn step(what: impl FnOnce() -> String) -> Option<Step> {
    on().then(|| Step { what: what(), at: Instant::now() })
}

impl Drop for Step {
    fn drop(&mut self) {
        let took = self.at.elapsed().as_secs_f64() * 1000.;
        mark(&format!("{} ({took:.1} ms)", self.what));
    }
}
