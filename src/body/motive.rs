use glam::DVec3;

/// A Motive is a method by which a body can move.
pub(crate) enum Motive {
    Fixed(FixedMotive),
    Linear(LinearMotive),
    StupidCircle(StupidCircle),
}

pub(crate) struct FixedMotive {
    pub(crate) local_position: DVec3,
}

impl Default for FixedMotive {
    fn default() -> Self {
        FixedMotive {
            local_position: DVec3::ZERO,
        }
    }
}

pub(crate) struct LinearMotive {
    pub(crate) local_position: DVec3,
    pub(crate) local_velocity: DVec3,
}

impl Default for LinearMotive {
    fn default() -> Self {
        LinearMotive {
            local_position: DVec3::ZERO,
            local_velocity: DVec3::new(1.0, 0.0, 0.0),
        }
    }
}

pub(crate) struct StupidCircle {
    pub(crate) radius: f64,
}

impl Default for StupidCircle {
    fn default() -> Self {
        StupidCircle {
            radius: 1.0,
        }
    }
}
