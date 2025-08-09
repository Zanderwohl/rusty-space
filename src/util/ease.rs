
pub enum Ease {
    Circ,
    Quad,
    Cubic,
    Sine,
    Expo,
}

pub mod f32 {
    use crate::util::ease::Ease;

    pub fn ease(t: f32, kind: Ease) -> f32 {
        match kind {
            Ease::Circ => circ(t),
            Ease::Quad => quad(t),
            Ease::Cubic => cubic(t),
            Ease::Sine => sine(t),
            Ease::Expo => expo(t),
        }
    }

    pub fn circ(t: f32) -> f32 {
        if t < 0.5 {
            (1.0 - (1.0 - (2.0 * t).powi(2)).sqrt()) / 2.0
        } else {
            ((1.0 - (-2.0 * t + 2.0).powi(2)).sqrt() + 1.0) / 2.0
        }
    }

    fn quad(t: f32) -> f32 {
        if t < 0.5 {
            2.0 * t * t
        } else {
            -1.0 + (4.0 - 2.0 * t) * t
        }
    }

    fn cubic(t: f32) -> f32 {
        if t < 0.5 {
            4.0 * t * t * t
        } else {
            (t - 1.0) * (2.0 * t - 2.0).powi(2) + 1.0
        }
    }

    fn sine(t: f32) -> f32 {
        -((std::f32::consts::PI * t).cos() - 1.0) / 2.0
    }

    fn expo(t: f32) -> f32 {
        if t == 0.0 || t == 1.0 {
            t
        } else if t < 0.5 {
            (2.0_f32).powf(20.0 * t - 10.0) / 2.0
        } else {
            (2.0 - (2.0_f32).powf(-20.0 * t + 10.0)) / 2.0
        }
    }

    fn smoothstep(t: f32) -> f32 {
        t * t * (3.0 - 2.0 * t)
    }
}

pub mod f64 {
    use crate::util::ease::Ease;

    pub fn ease(t: f64, kind: Ease) -> f64 {
        match kind {
            Ease::Circ => circ(t),
            Ease::Quad => quad(t),
            Ease::Cubic => cubic(t),
            Ease::Sine => sine(t),
            Ease::Expo => expo(t),
        }
    }

    pub fn circ(t: f64) -> f64 {
        if t < 0.5 {
            (1.0 - (1.0 - (2.0 * t).powi(2)).sqrt()) / 2.0
        } else {
            ((1.0 - (-2.0 * t + 2.0).powi(2)).sqrt() + 1.0) / 2.0
        }
    }

    fn quad(t: f64) -> f64 {
        if t < 0.5 {
            2.0 * t * t
        } else {
            -1.0 + (4.0 - 2.0 * t) * t
        }
    }

    fn cubic(t: f64) -> f64 {
        if t < 0.5 {
            4.0 * t * t * t
        } else {
            (t - 1.0) * (2.0 * t - 2.0).powi(2) + 1.0
        }
    }

    fn sine(t: f64) -> f64 {
        -((std::f64::consts::PI * t).cos() - 1.0) / 2.0
    }

    fn expo(t: f64) -> f64 {
        if t == 0.0 || t == 1.0 {
            t
        } else if t < 0.5 {
            (2.0_f64).powf(20.0 * t - 10.0) / 2.0
        } else {
            (2.0 - (2.0_f64).powf(-20.0 * t + 10.0)) / 2.0
        }
    }

    fn smoothstep(t: f64) -> f64 {
        t * t * (3.0 - 2.0 * t)
    }
}