use glam::DVec3;

pub trait Body {
    fn global_position(&self) -> DVec3;

    fn global_position_after_time(&self, delta: f64) -> DVec3;

    fn mass(&self) -> f64;
}

pub struct BodyProperties {
    pub(crate) mass: f64
}
