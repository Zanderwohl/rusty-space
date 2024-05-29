// Surely nothing can go wrong with a little bit nonsense.

pub mod f64 {
    use std::mem::transmute;

    fn to_i64(value: f64) -> i64 {
        unsafe { transmute(value) }
    }
}

pub mod i64 {
    use std::mem::transmute;

    fn i64_to_f64(value: i64) -> f64 {
        unsafe { transmute(value) }
    }
}
