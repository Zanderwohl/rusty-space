//! How often someone may get a password wrong.
//!
//! Not deferred, unlike delivery and captcha. With neither of those, an attempt budget is the
//! only thing between a password provider and credential stuffing — see
//! `lightcone/docs/16-identity.md`.
//!
//! In memory, because the broker is one container. A second instance needs this in the
//! database, and the shape here is the shape that table would have.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Failures allowed against one account before it stops answering.
///
/// Low: a person who has forgotten their password tries three or four times, and with reset
/// deferred there is no flow this locks anyone out of that they had.
pub const PER_ACCOUNT: u32 = 10;

/// Failures allowed from one address, across every account it tries.
///
/// Higher, because a household or an office is one address, and lower than it looks: stuffing a
/// leaked credential list means one attempt per account, which this counts and the per-account
/// budget never sees.
pub const PER_ADDRESS: u32 = 50;

/// How long a budget takes to refill.
pub const WINDOW: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy)]
struct Window {
    failures: u32,
    started: Instant,
}

#[derive(Default)]
pub struct Attempts {
    counted: Mutex<HashMap<String, Window>>,
}

impl Attempts {
    /// Whether `key` may try again, at `now`.
    pub fn allows_at(&self, key: &str, limit: u32, now: Instant) -> bool {
        let counted = self.counted.lock().unwrap();
        match counted.get(key) {
            Some(window) if now.duration_since(window.started) < WINDOW => window.failures < limit,
            _ => true,
        }
    }

    /// Record a failure. Only failures are counted: a working sign-in costs nothing, so a busy
    /// account is not a throttled one.
    pub fn failed_at(&self, key: &str, now: Instant) {
        let mut counted = self.counted.lock().unwrap();
        // Expired entries are dropped here rather than by a sweeper. The map is only as large
        // as the accounts that failed recently, and every one of them is touched again to grow.
        counted.retain(|_, window| now.duration_since(window.started) < WINDOW);
        let window = counted.entry(key.to_owned()).or_insert(Window {
            failures: 0,
            started: now,
        });
        window.failures += 1;
    }

    /// Forget a key. Called on a successful sign-in, so someone who remembers their password on
    /// the ninth try is not left one attempt from a lockout for the next quarter of an hour.
    pub fn cleared(&self, key: &str) {
        self.counted.lock().unwrap().remove(key);
    }

    pub fn allows(&self, key: &str, limit: u32) -> bool {
        self.allows_at(key, limit, Instant::now())
    }

    pub fn failed(&self, key: &str) {
        self.failed_at(key, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_runs_out_and_then_refills() {
        let attempts = Attempts::default();
        let start = Instant::now();
        for _ in 0..PER_ACCOUNT {
            assert!(attempts.allows_at("ada", PER_ACCOUNT, start));
            attempts.failed_at("ada", start);
        }
        assert!(
            !attempts.allows_at("ada", PER_ACCOUNT, start),
            "the budget did not run out"
        );

        // Still out most of the way through the window, and back at the end of it.
        assert!(!attempts.allows_at("ada", PER_ACCOUNT, start + WINDOW - Duration::from_secs(1)));
        assert!(attempts.allows_at("ada", PER_ACCOUNT, start + WINDOW));
    }

    /// Only failures count. Otherwise an account someone is actively using throttles itself.
    #[test]
    fn a_successful_sign_in_clears_the_count() {
        let attempts = Attempts::default();
        let now = Instant::now();
        for _ in 0..PER_ACCOUNT - 1 {
            attempts.failed_at("ada", now);
        }
        attempts.cleared("ada");
        for _ in 0..PER_ACCOUNT - 1 {
            assert!(attempts.allows_at("ada", PER_ACCOUNT, now));
            attempts.failed_at("ada", now);
        }
    }

    /// Budgets are per key, so one account being attacked does not lock out another.
    #[test]
    fn keys_are_counted_separately() {
        let attempts = Attempts::default();
        let now = Instant::now();
        for _ in 0..PER_ACCOUNT {
            attempts.failed_at("account:ada", now);
        }
        assert!(!attempts.allows_at("account:ada", PER_ACCOUNT, now));
        assert!(attempts.allows_at("account:grace", PER_ACCOUNT, now));
        // And the same key can be held to a different limit, which is how one address gets a
        // looser budget than one account.
        assert!(attempts.allows_at("account:ada", PER_ADDRESS, now));
    }

    /// The map must not grow with every address that ever failed once.
    #[test]
    fn expired_entries_are_dropped() {
        let attempts = Attempts::default();
        let start = Instant::now();
        for i in 0..100 {
            attempts.failed_at(&format!("addr:{i}"), start);
        }
        assert_eq!(attempts.counted.lock().unwrap().len(), 100);
        attempts.failed_at("addr:later", start + WINDOW + Duration::from_secs(1));
        assert_eq!(
            attempts.counted.lock().unwrap().len(),
            1,
            "the old windows were kept"
        );
    }
}
