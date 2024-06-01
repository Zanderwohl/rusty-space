use serde::{Deserialize, Serialize};
use crate::body::appearance::{Appearance, Planetoid};
use crate::body::motive::{Motive, MotiveTypes};
use crate::body::motive::fixed::FixedMotive;

#[derive(Serialize, Deserialize, Debug)]
pub struct Body {
    /// ID in Body's universe. Assigned by Universe. Should be unique, can change with saves/loads.
    pub(crate) id: Option<u32>,
    pub(crate) physics: MotiveTypes,
    pub(crate) name: String,
    pub(crate) mass: f64,
    pub(crate) appearance: Appearance,
}

impl Body {
    pub fn get_name(&self) -> String {
        self.name.clone()
    }

    pub fn primary(&self) -> Option<u32> {
        self.motive_ref().defined_primary()
    }

    pub fn motive_ref(&self) -> Box<&dyn Motive> {
        match &self.physics {
            MotiveTypes::Fixed(fixed_motive) => Box::new(fixed_motive),
            MotiveTypes::Linear(linear_motive) => Box::new(linear_motive),
            MotiveTypes::StupidCircle(stupid_circle) => Box::new(stupid_circle),
            MotiveTypes::FlatKepler(flat_kepler) => Box::new(flat_kepler),
        }
    }

    pub fn motive_mut(&mut self) -> Box<&mut dyn Motive> {
        match &mut self.physics {
            MotiveTypes::Fixed(ref mut fixed_motive) => Box::new(fixed_motive),
            MotiveTypes::Linear(ref mut linear_motive) => Box::new(linear_motive),
            MotiveTypes::StupidCircle(ref mut stupid_circle) => Box::new(stupid_circle),
            MotiveTypes::FlatKepler(ref mut flat_kepler) => Box::new(flat_kepler),
        }
    }
}

impl Default for Body {
    fn default() -> Self {
        Body {
            id: None,
            physics: MotiveTypes::Fixed(FixedMotive::default()),
            name: "New body".to_string(),
            mass: 1.0,
            appearance: Appearance::Planetoid(Planetoid {
                radius: 1.0,
                color: [0.3, 0.3, 0.3],
            }),
        }
    }
}
