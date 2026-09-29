//! The encoding of every record the shard stores: CBOR, where the wire is postcard.
//!
//! Stored bytes outlive the process that wrote them, so the format has to name its fields: a
//! field appended to a record then reads as its `#[serde(default)]` from an older row, where
//! positional postcard would misread or refuse every row there is. The wire can stay positional
//! because a client and its shard are deployed together; see `lightcone/docs/08-networking.md`.
//!
//! Exact, like postcard. An f64 is written in the shortest of f16, f32 and f64 that converts
//! back to the same bits, and otherwise as its eight bytes, so nothing written is perturbed.
//! JSON is not, which is why it was never the answer: see [`crate::persist`].

use serde::{Serialize, de::DeserializeOwned};

pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    // Writing to a Vec cannot fail, and no stored type has a serializer that errors.
    #[allow(clippy::expect_used)]
    ciborium::into_writer(value, &mut bytes).expect("a stored record encodes");
    bytes
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    ciborium::from_reader(bytes).map_err(|why| why.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn back(value: f64) -> f64 {
        decode(&encode(&value)).unwrap()
    }

    /// Every awkward f64 comes back to the bit, including those ciborium shrinks on the way.
    #[test]
    fn floats_round_trip_to_the_bit() {
        let awkward = [
            -1.8149592025296526e-22,
            4.200079062537049,
            0.0,
            -0.0,
            1.0,
            0.5,
            65504.0,
            f64::MIN_POSITIVE,
            // Subnormal, smallest and largest.
            f64::from_bits(1),
            f64::from_bits(0x000f_ffff_ffff_ffff),
            -f64::from_bits(7),
            f64::MAX,
            f64::MIN,
            f64::EPSILON,
            // Orbits state an unbounded element as infinity.
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            -f64::NAN,
            // A NaN with a payload, which no narrower width can carry.
            f64::from_bits(0x7ff8_0000_dead_beef),
        ];
        for value in awkward {
            assert_eq!(back(value).to_bits(), value.to_bits(), "{value:e} ({:#018x})", value.to_bits());
        }
        for value in [0.25f32, f32::from_bits(1), 1.0e-40, f32::INFINITY, f32::NAN, 3.4028235e38] {
            let read: f32 = decode(&encode(&value)).unwrap();
            assert_eq!(read.to_bits(), value.to_bits(), "{value:e}");
        }
    }

    /// Shrinking is what keeps small numbers small, and only exact values are shrunk.
    #[test]
    fn a_float_is_shrunk_only_when_exact() {
        assert_eq!(encode(&1.5f64).len(), 3, "an f16");
        assert_eq!(encode(&0.1f32).len(), 5, "an f32");
        assert_eq!(encode(&0.1f64).len(), 9, "the whole f64");
        assert_eq!(encode(&f64::from_bits(1)).len(), 9, "a subnormal no narrower width holds");
        assert_eq!(encode(&f64::from_bits(0x7ff8_0000_dead_beef)).len(), 9, "a payload");
    }
}
