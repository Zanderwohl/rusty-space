//! Event identifiers.
//!
//! Snowflake-shaped: a coordinate second, a shard and a sequence, packed into the `bigint` the
//! column is. Locally generated, so writing an event needs no round trip and no coordination,
//! and time-ordered, so an append lands at the end of the index instead of in the middle of it.
//! It works unchanged whether or not sharding ever happens — see `lightcone/docs/02-event-store.md`.

/// Bits for the sequence within one shard and one coordinate second.
pub const SEQUENCE_BITS: u32 = 14;
/// Bits for the shard.
pub const SHARD_BITS: u32 = 10;
/// What is left for the time, out of the 63 a positive `bigint` has.
pub const TIME_BITS: u32 = 63 - SEQUENCE_BITS - SHARD_BITS;

pub const MAX_SEQUENCE: u64 = (1 << SEQUENCE_BITS) - 1;
pub const MAX_SHARD: u64 = (1 << SHARD_BITS) - 1;
/// The last coordinate second an identifier can carry: about seventeen thousand years.
pub const MAX_SECOND: i64 = (1i64 << TIME_BITS) - 1;

/// Microseconds in a second. The store's times are microseconds; identifiers are seconds.
pub const MICROS_PER_SECOND: i64 = 1_000_000;

/// A packed event identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(i64);

impl EventId {
    /// Pack one. `None` when any part is out of range, which is a programming error rather
    /// than something to paper over: a truncated identifier collides silently.
    pub fn new(second: i64, shard: u64, sequence: u64) -> Option<Self> {
        if !(0..=MAX_SECOND).contains(&second) || shard > MAX_SHARD || sequence > MAX_SEQUENCE {
            return None;
        }
        Some(Self(
            (second << (SHARD_BITS + SEQUENCE_BITS)) | ((shard as i64) << SEQUENCE_BITS) | sequence as i64,
        ))
    }

    pub fn get(self) -> i64 {
        self.0
    }

    pub fn from_raw(raw: i64) -> Option<Self> {
        (raw >= 0).then_some(Self(raw))
    }

    pub fn second(self) -> i64 {
        self.0 >> (SHARD_BITS + SEQUENCE_BITS)
    }

    pub fn shard(self) -> u64 {
        ((self.0 >> SEQUENCE_BITS) as u64) & MAX_SHARD
    }

    pub fn sequence(self) -> u64 {
        (self.0 as u64) & MAX_SEQUENCE
    }
}

/// Hands out identifiers for one shard, in order.
#[derive(Clone, Debug)]
pub struct Minter {
    shard: u64,
    second: i64,
    sequence: u64,
}

impl Minter {
    /// `None` for a shard outside the range the packing allows.
    pub fn new(shard: u64) -> Option<Self> {
        (shard <= MAX_SHARD).then_some(Self { shard, second: i64::MIN, sequence: 0 })
    }

    /// The next identifier for an event at coordinate time `t_micros`.
    ///
    /// Never blocks. A shard that produces more than sixteen thousand events inside one
    /// coordinate second borrows from the next one — the identifier then runs slightly ahead of
    /// the event's own time, which costs nothing because `t` is stored in its own column and is
    /// what every query and every partition uses. Blocking instead would be worse: coordinate
    /// time is the simulation's, not the wall clock's, and waiting for it to advance would stop
    /// the thing that advances it.
    pub fn mint(&mut self, t_micros: i64) -> Option<EventId> {
        let second = t_micros.div_euclid(MICROS_PER_SECOND);
        if second > self.second {
            self.second = second;
            self.sequence = 0;
        } else if self.sequence > MAX_SEQUENCE {
            self.second += 1;
            self.sequence = 0;
        }
        let id = EventId::new(self.second, self.shard, self.sequence)?;
        self.sequence += 1;
        Some(id)
    }

    pub fn shard(&self) -> u64 {
        self.shard
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_parts_survive_the_packing() {
        let id = EventId::new(1_234_567, 513, 9_999).expect("in range");
        assert_eq!(id.second(), 1_234_567);
        assert_eq!(id.shard(), 513);
        assert_eq!(id.sequence(), 9_999);
        assert!(id.get() > 0, "a bigint identifier has to stay positive");
    }

    #[test]
    fn the_extremes_pack_and_anything_past_them_refuses() {
        let top = EventId::new(MAX_SECOND, MAX_SHARD, MAX_SEQUENCE).expect("the last one");
        assert_eq!((top.second(), top.shard(), top.sequence()), (MAX_SECOND, MAX_SHARD, MAX_SEQUENCE));
        assert!(top.get() > 0 && top.get() == i64::MAX, "the packing should fill the range");

        assert!(EventId::new(MAX_SECOND + 1, 0, 0).is_none());
        assert!(EventId::new(0, MAX_SHARD + 1, 0).is_none());
        assert!(EventId::new(0, 0, MAX_SEQUENCE + 1).is_none());
        assert!(EventId::new(-1, 0, 0).is_none());
    }

    /// Seventeen thousand years of coordinate time, which is what the field has to hold for a
    /// world that runs at eight thousand times real time for a decade.
    #[test]
    fn the_time_field_outlives_the_game() {
        let years = MAX_SECOND as f64 / 31_557_600.0;
        assert!(years > 10_000.0, "only {years} years of identifiers");
    }

    /// Identifiers rise with time, which is what makes an append land at the end of the index.
    #[test]
    fn identifiers_from_one_shard_only_ever_increase() {
        let mut minter = Minter::new(7).unwrap();
        let mut last = EventId::new(0, 0, 0).unwrap();
        for step in 0..1000i64 {
            let id = minter.mint(step * 3_000).expect("an identifier");
            assert!(id > last, "{id:?} did not follow {last:?}");
            assert_eq!(id.shard(), 7);
            last = id;
        }
    }

    /// Two shards never collide, whatever they are doing.
    #[test]
    fn two_shards_cannot_produce_the_same_identifier() {
        let (mut a, mut b) = (Minter::new(1).unwrap(), Minter::new(2).unwrap());
        let mut seen = std::collections::HashSet::new();
        for step in 0..500i64 {
            assert!(seen.insert(a.mint(step * 1_000).unwrap()));
            assert!(seen.insert(b.mint(step * 1_000).unwrap()));
        }
        assert_eq!(seen.len(), 1000);
    }

    /// A burst past the sequence field borrows from the next second rather than colliding.
    /// The identifier runs ahead of the event's time; `t` is a column of its own and is what
    /// partitions and queries use, so nothing downstream notices.
    #[test]
    fn a_burst_past_the_sequence_rolls_forward_instead_of_repeating() {
        let mut minter = Minter::new(0).unwrap();
        let mut seen = std::collections::HashSet::new();
        let burst = (MAX_SEQUENCE as usize + 1) * 3;
        let mut last = None;
        for _ in 0..burst {
            let id = minter.mint(5_000_000).expect("an identifier");
            assert!(seen.insert(id), "{id:?} came out twice");
            if let Some(previous) = last {
                assert!(id > previous, "and out of order");
            }
            last = Some(id);
        }
        assert_eq!(seen.len(), burst);
        // It really did run ahead, which is the cost being accepted.
        assert!(last.unwrap().second() > 5, "the roll-forward never happened");
    }

    /// Time going backwards -- a replay, or a clock correction -- must not repeat an identifier
    /// already handed out.
    #[test]
    fn a_clock_that_goes_backwards_does_not_repeat() {
        let mut minter = Minter::new(3).unwrap();
        let first = minter.mint(10_000_000).unwrap();
        let second = minter.mint(1_000_000).unwrap();
        assert_ne!(first, second);
        assert!(second > first, "the sequence carries it, not the clock");
    }

    #[test]
    fn a_shard_outside_the_field_refuses_to_exist() {
        assert!(Minter::new(MAX_SHARD).is_some());
        assert!(Minter::new(MAX_SHARD + 1).is_none());
    }

    #[test]
    fn a_raw_identifier_round_trips_and_a_negative_one_refuses() {
        let id = EventId::new(42, 3, 7).unwrap();
        assert_eq!(EventId::from_raw(id.get()), Some(id));
        assert!(EventId::from_raw(-1).is_none());
    }
}
