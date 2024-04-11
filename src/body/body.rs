use glam::DVec3;

#[bevy_trait_query::queryable]
pub trait Body {
    fn local_position(&self) -> DVec3;

    fn local_position_after_time(&self, delta: f64) -> DVec3;

    fn mass(&self) -> f64;

    fn name(&self) -> &String;

    fn mu(&self) -> f64 {
        self.mass() * 6.6743015e-11f64
    }

    fn size(&self) -> f64;
}

pub struct BodyProperties {
    pub(crate) mass: f64,
    pub(crate) name: String,
    pub(crate) size: f64,
}
