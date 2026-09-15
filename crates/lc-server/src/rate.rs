//! What a client is allowed to send.
//!
//! This is a **safety valve, not a game rule.** It exists so that a scripted client cannot
//! consume the server, and it is set deliberately far above any plausible legitimate rate. The
//! real cost of an action belongs in the fiction — reaction mass for a burn, energy for a
//! transmission, a cooldown on an instrument — which is where
//! `lightcone/docs/02-event-store.md` already puts it: *a transmitter that raises its power
//! raises its fan-out cost, which is a fair place for a game-balance knob to live.*
//!
//! Nobody has measured what a real player sends, because there is not yet a game to measure.
//! So this counts, and [`Usage`] is what will one day replace the guess with evidence: if the
//! peak a real session reaches is nowhere near the ceiling, the ceiling is fine and the number
//! can stop being provisional. If a legitimate client ever trips it, that is a bug report with
//! the measurement already attached.

/// Messages per real second a client may sustain.
///
/// Anchored to the store's own budget rather than to taste. `02-event-store.md` sizes the event
/// table on *ten thousand players producing one event per real second*, so one per second is
/// the rate the storage was designed for and this is double it. It is not frame-coupled and
/// should not be: a burn is one intent that then runs for days of coordinate time, an order is
/// one intent, and the ship's motion in between is analytic and predicted on the client. This
/// is a strategy game's input rate, not a shooter's.
pub const SUSTAINED_PER_SECOND: f64 = 2.0;

/// How many may arrive at once, for a player doing several things in a moment.
///
/// Fifteen seconds of the sustained rate. Large enough that no burst of deliberate orders can
/// reach it; small enough that a runaway client is stopped within a tick or two.
pub const BURST: f64 = 30.0;

/// What a client actually sent. The reason this module counts at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub accepted: u64,
    pub refused: u64,
    /// The most this client sent in any single tick.
    pub peak_per_tick: u32,
    /// The most it sent across any one second, which is what the ceiling is written in.
    pub peak_per_second: u32,
    within_tick: u32,
    within_second: u32,
    ticks_this_second: u32,
}

/// Thousandths of a token, which is what the bucket actually counts in.
///
/// Integer, not floating point. A bucket refilled by a tenth of a token a tick and asked to
/// wait ten ticks got back 0.9999999999999999 and refused a client that had waited exactly as
/// long as it was told to. There is no tolerance that fixes that honestly; there is an integer
/// that removes it.
const SCALE: i64 = 1_000;

/// One client's allowance: a token bucket, refilled a little every tick.
#[derive(Clone, Debug)]
pub struct Budget {
    tokens: i64,
    per_tick: i64,
    capacity: i64,
    pub usage: Usage,
}

impl Budget {
    /// An allowance for a server ticking every `tick_ms` real milliseconds.
    pub fn new(tick_ms: i64) -> Self {
        let capacity = (BURST * SCALE as f64).round() as i64;
        Self {
            tokens: capacity,
            // Rounded once, here, and exact from then on.
            per_tick: ((SUSTAINED_PER_SECOND * tick_ms as f64 / 1000.0) * SCALE as f64).round()
                as i64,
            capacity,
            usage: Usage::default(),
        }
    }

    /// A tick has passed: refill, and close the window the counters were measuring.
    ///
    /// Refill is per tick rather than per elapsed wall second, so a server running slow limits
    /// proportionally harder. That is the behaviour wanted under load rather than a defect: the
    /// thing being protected is the server, and a server that is struggling should take less.
    pub fn advance(&mut self, ticks_per_second: u32) {
        self.tokens = (self.tokens + self.per_tick).min(self.capacity);
        self.usage.peak_per_tick = self.usage.peak_per_tick.max(self.usage.within_tick);
        self.usage.within_second += self.usage.within_tick;
        self.usage.within_tick = 0;
        self.usage.ticks_this_second += 1;
        if self.usage.ticks_this_second >= ticks_per_second.max(1) {
            self.usage.peak_per_second =
                self.usage.peak_per_second.max(self.usage.within_second);
            self.usage.within_second = 0;
            self.usage.ticks_this_second = 0;
        }
    }

    /// Take one, if there is one. Charged before the message is read, so an invalid message
    /// costs the sender as much as a valid one.
    pub fn charge(&mut self) -> bool {
        self.usage.within_tick += 1;
        if self.tokens < SCALE {
            self.usage.refused += 1;
            return false;
        }
        self.tokens -= SCALE;
        self.usage.accepted += 1;
        true
    }

    /// Ticks until the next message would be taken. Told to the client so it can back off
    /// rather than spin.
    pub fn retry_after_ticks(&self) -> u32 {
        if self.tokens >= SCALE || self.per_tick <= 0 {
            return 0;
        }
        // Ceiling division, in integers, so the answer is exactly long enough.
        let short = SCALE - self.tokens;
        ((short + self.per_tick - 1) / self.per_tick) as u32
    }

    /// How full it is, in whole tokens, for a test or a readout.
    pub fn tokens(&self) -> f64 {
        self.tokens as f64 / SCALE as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK_MS: i64 = 50;
    const TICKS_PER_SECOND: u32 = 20;

    /// The sustained rate is what it says: a client sending at it forever is never refused.
    #[test]
    fn the_sustained_rate_runs_for_ever() {
        let mut budget = Budget::new(TICK_MS);
        // Two a second for five minutes, spread evenly.
        let every = TICKS_PER_SECOND / SUSTAINED_PER_SECOND as u32;
        for tick in 0..(TICKS_PER_SECOND * 300) {
            if tick % every == 0 {
                assert!(budget.charge(), "refused at the sustained rate on tick {tick}");
            }
            budget.advance(TICKS_PER_SECOND);
        }
        assert_eq!(budget.usage.refused, 0);
    }

    /// A burst of deliberate orders goes through at once. A player does several things in a
    /// moment and should not be told to wait.
    #[test]
    fn a_burst_of_orders_goes_through_at_once() {
        let mut budget = Budget::new(TICK_MS);
        for step in 0..BURST as u32 {
            assert!(budget.charge(), "refused order {step} of a burst");
        }
        assert_eq!(budget.usage.accepted, BURST as u64);
    }

    /// And the one after the burst is not, which is the whole point.
    #[test]
    fn a_flood_is_stopped_and_told_when_to_come_back() {
        let mut budget = Budget::new(TICK_MS);
        for _ in 0..BURST as u32 {
            assert!(budget.charge());
        }
        assert!(!budget.charge(), "a flood was not stopped");
        let wait = budget.retry_after_ticks();
        assert!(wait > 0, "told to retry immediately after being refused");

        // And after that many ticks it really is taken.
        for _ in 0..wait {
            budget.advance(TICKS_PER_SECOND);
        }
        assert!(budget.charge(), "still refused after waiting the {wait} ticks it asked for");
    }

    /// It refills, and never past the burst: an idle client banks a burst, not an hour.
    #[test]
    fn an_idle_client_banks_a_burst_and_no_more() {
        let mut budget = Budget::new(TICK_MS);
        for _ in 0..BURST as u32 {
            budget.charge();
        }
        for _ in 0..(TICKS_PER_SECOND * 3600) {
            budget.advance(TICKS_PER_SECOND);
        }
        assert_eq!(budget.tokens(), BURST, "an hour idle banked more than a burst");
    }

    /// The measurement, which is the part that will make the ceiling stop being a guess.
    #[test]
    fn usage_records_the_peaks_the_ceiling_should_have_been_set_from() {
        let mut budget = Budget::new(TICK_MS);
        for _ in 0..7 {
            budget.charge();
        }
        budget.advance(TICKS_PER_SECOND);
        for _ in 0..3 {
            budget.charge();
        }
        for _ in 0..TICKS_PER_SECOND {
            budget.advance(TICKS_PER_SECOND);
        }
        assert_eq!(budget.usage.peak_per_tick, 7, "the busiest tick was seven");
        assert_eq!(budget.usage.peak_per_second, 10, "and the busiest second was ten");
        assert_eq!(budget.usage.accepted, 10);
        assert_eq!(budget.usage.refused, 0);
    }

    /// A refused message still counts as sent. What the ceiling wants to know is how fast a
    /// client tried, not how fast it succeeded.
    #[test]
    fn a_refusal_is_still_a_measurement() {
        let mut budget = Budget::new(TICK_MS);
        for _ in 0..(BURST as u32 + 5) {
            budget.charge();
        }
        budget.advance(TICKS_PER_SECOND);
        assert_eq!(budget.usage.accepted, BURST as u64);
        assert_eq!(budget.usage.refused, 5);
        assert_eq!(budget.usage.peak_per_tick, BURST as u32 + 5, "the attempt was not recorded");
    }

    /// A slower server takes less, which is the behaviour wanted under load.
    #[test]
    fn a_server_running_slow_limits_harder() {
        let fast = Budget::new(50);
        let slow = Budget::new(50);
        let mut fast = fast;
        let mut slow = slow;
        for _ in 0..BURST as u32 {
            fast.charge();
            slow.charge();
        }
        // Twenty ticks at 20 Hz is a second; twenty ticks at 10 Hz is two seconds of wall time
        // but the same twenty refills, so the same tokens. The rate is per tick by design.
        for _ in 0..20 {
            fast.advance(20);
            slow.advance(10);
        }
        assert_eq!(fast.tokens(), slow.tokens(), "refill should follow ticks, not wall time");
    }
}
