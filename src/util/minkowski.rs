use bevy::math::DVec4;
use std::ops::{Add, Sub};

pub struct Minkowski {
    vals: DVec4,
}

pub enum Classification {
    Lightlike,
    Timelike,
    Spacelike,
}

pub enum Relation {
    Simultaneous, // Other appears at same time in self's frame
    Future,
    Past,
    Lightlike, // Other can be observed by self; light signals arriving now
    SpaceLike,
}

// For these purposes, c = 1
impl Minkowski {
    pub fn new(x: f64, y: f64, z: f64, t: f64) -> Self {
        Self {
            vals: DVec4::new(x, y, z, t)
        }
    }

    pub fn minkowski_dot(&self, other: &Self) -> f64 {
        let a = &self.vals;
        let b = &other.vals;
        a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z
    }

    pub fn minkowski_absolute_length(&self) -> f64 {
        self.minkowski_dot(self).abs().sqrt()
    }

    pub fn minkowski_signed_length(&self) -> f64 {
        let interval = self.minkowski_dot(self);
        interval.abs().sqrt() * interval.signum()
    }

    pub fn classify(&self) -> Classification {
        let interval = self.minkowski_dot(self);
        if interval > 0.0 {
            Classification::Timelike
        } else if interval < 0.0 {
            Classification::Spacelike
        } else {
            Classification::Lightlike
        }
    }

    pub fn distance(&self, other: &Self) -> f64 {
        let delta = self.vals - other.vals;
        let interval = delta.w * delta.w - delta.x * delta.x - delta.y * delta.y - delta.z * delta.z;
        interval.abs().sqrt() * interval.signum()
    }

    fn relation(&self, other: &Self) -> Relation {
        let delta = self.vals - other.vals;
        let interval = delta.w * delta.w - delta.x * delta.x - delta.y * delta.y - delta.z * delta.z;
        if delta.w == 0.0 {
            Relation::Simultaneous
        } else if delta.w > 0.0 {
            if interval > 0.0 {
                Relation::Future
            } else {
                Relation::Past
            }
        } else {
            if interval > 0.0 {
                Relation::Lightlike
            } else {
                Relation::SpaceLike
            }
        }
    }

    /// Returns the relation between two events and the "fuzziness" of the relation.
    /// Fuzziness is a value between 0 and 1.
    /// 1 means the relation is precise, and lower values mean a slight distance.
    fn relation_fuzzy(&self, other: &Self, epsilon: f64) -> (Relation, f64) {
        let delta = self.vals - other.vals;
        let interval = delta.w * delta.w - delta.x * delta.x - delta.y * delta.y - delta.z * delta.z;
        if delta.w.abs() < epsilon {
            (Relation::Simultaneous, delta.w.abs() / epsilon)
        } else if interval.abs() < epsilon {
            (Relation::Lightlike, interval.abs() / epsilon)
        } else if delta.w > 0.0 {
            if interval > 0.0 {
                (Relation::Future, 0.0)
            } else {
                (Relation::Past, 0.0)
            }
        } else {
            (Relation::SpaceLike, 0.0)
        }
    }
}

impl Add for Minkowski {
    type Output = Self;

    fn add(self, other: Self) -> Self::Output {
        Self {
            vals: self.vals + other.vals
        }
    }
}

impl Add<&Minkowski> for &Minkowski {
    type Output = Minkowski;

    fn add(self, other: &Minkowski) -> Self::Output {
        Minkowski {
            vals: self.vals + other.vals
        }
    }
}

impl Sub for Minkowski {
    type Output = Self;

    fn sub(self, other: Self) -> Self::Output {
        Self {
            vals: self.vals - other.vals
        }
    }
}

impl Sub<&Minkowski> for &Minkowski {
    type Output = Minkowski;

    fn sub(self, other: &Minkowski) -> Self::Output {
        Minkowski {
            vals: self.vals - other.vals
        }
    }
}
