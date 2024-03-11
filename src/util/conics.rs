fn semi_minor_axis(semi_major_axis: f64, eccentricity: f64) -> f64 {
    semi_major_axis * f64::sqrt(1 - (eccentricity * eccentricity))
}
