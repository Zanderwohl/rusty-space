use crate::body::appearance::{Appearance, Planetoid};
use crate::body::motive::{FixedMotive, Motive};

pub struct Body {
    pub(crate) physics: Motive,
    pub(crate) name: String,
    pub(crate) mass: f64,
    pub(crate) appearance: Appearance,
    pub(crate) parent: Option<u32>,
}

impl Default for Body {
    fn default() -> Self {
        Body {
            physics: Motive::Fixed(FixedMotive::default()),
            name: "New body".to_string(),
            mass: 1.0,
            parent: None,
            appearance: Appearance::Planetoid(Planetoid {
                radius: 1.0,
                color: [0.3, 0.3, 0.3],
            })
        }
    }
}
