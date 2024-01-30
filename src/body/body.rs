use glam::Vec3;

pub trait Body {
    fn global_position(&self) -> Vec3;

    fn time_step(&mut self, delta: i128);
}

pub struct BodyProperties {
    mass_grams: u128
}
