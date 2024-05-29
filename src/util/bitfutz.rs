// Surely nothing can go wrong with a little bit nonsense.

pub mod f64 {
    use std::mem::transmute;

    pub fn to_u64(value: f64) -> u64 {
        unsafe { transmute(value) }
    }
}

pub mod u64 {
    use std::mem::transmute;

    pub fn to_f64(value: u64) -> f64 {
        unsafe { transmute(value) }
    }
}
