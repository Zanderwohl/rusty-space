use num_traits::Float;

pub fn log_scale<T: Float>(value: T, base: T) -> T {
    (value + (T::from(1.0).unwrap() / base)).log(base) + T::from(1.0).unwrap()
}

/// Scales a value using sigmoid, normalizing to [0, 1]
pub fn sigmoid_scale<T: Float>(value: T) -> T {
    T::from(1.0).unwrap() / (T::from(1.0).unwrap() + (-value).exp())
}

pub fn bound_circle<T: Float>(value: T, max: T) -> T {
    let value = value % max;
    (value + max) % max
}

pub fn bound_degrees<T: Float>(value: T, min: T) -> T {
    bound_circle(min, T::from(360.0).unwrap())
}
