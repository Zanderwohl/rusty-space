use glam::DVec3;

fn distance(focal_parameter: f64, eccentricity: f64, true_anomaly: f64) -> f64 {
    let numerator = focal_parameter * eccentricity;
    let denominator = 1 + eccentricity * f64::cos(true_anomaly);
    numerator / denominator
}

fn mean_angular_motion(gravitational_parameter: f64, semi_major_axis: f64) -> f64 {
    f64::sqrt(gravitational_parameter / (semi_major_axis * semi_major_axis * semi_major_axis))
}


fn eccentric_anomaly() -> DVec3 {
    todo!()
}

fn mean_anomaly() {

}
