use num_traits::Float;

pub fn log_scale<T: Float>(value: T, base: T) -> T {
    (value + (T::from(1.0).unwrap() / base)).log(base) + T::from(1.0).unwrap()
}

/// Scales a value using arctangent, normalizing to [-1, 1]
pub fn arctan_scale<T: Float>(value: T) -> T {
    let pi = T::from(std::f64::consts::PI).unwrap();
    value.atan() * (T::from(2.0).unwrap() / pi)
}

/// Scales a value using hyperbolic tangent, normalizing to [-1, 1]
pub fn tanh_scale<T: Float>(value: T) -> T {
    value.tanh()
}

/// Scales a value using sigmoid, normalizing to [0, 1]
pub fn sigmoid_scale<T: Float>(value: T) -> T {
    T::from(1.0).unwrap() / (T::from(1.0).unwrap() + (-value).exp())
}

/// A custom piecewise function that's linear near zero and logarithmic for large values
pub fn smooth_log_scale<T: Float>(value: T, transition: T) -> T {
    let one = T::from(1.0).unwrap();
    let two = T::from(2.0).unwrap();
    
    if value.abs() <= transition {
        // Linear region near zero
        value / transition
    } else {
        // Logarithmic region for large values
        let sign = value.signum();
        sign * (one + value.abs().log(one + transition))
    }
}
