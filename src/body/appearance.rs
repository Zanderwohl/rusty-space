pub(crate) enum Appearance {
    Planetoid(Planetoid),
    Sun(Sun),
    Ring(Ring),
}

pub(crate) struct Planetoid {
    pub radius: f64,
    pub color: [f32; 3],
}

pub(crate) struct Sun {
    pub radius: f64,
    pub color: [f32; 3],
    pub light: [f32; 3],
    pub brightness: f32,
}

pub(crate) struct Ring {
    pub radius: f64,
    pub thickness: f64,
    pub width: f64,
    pub wall_height: f64,
}
