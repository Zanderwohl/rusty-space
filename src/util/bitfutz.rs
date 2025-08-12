// Surely nothing can go wrong with a little bit nonsense.

pub mod f64 {
    pub fn to_u64(value: f64) -> u64 {
        f64::to_bits(value)
    }
}

pub mod u64 {
    pub fn to_f64(value: u64) -> f64 {
        f64::from_bits(value)
    }
}
