use serde::{Serialize, Deserialize};
use crate::body::appearance::{Appearance, Planetoid};
use crate::body::motive::{FixedMotive, Motive};

#[derive(Serialize, Deserialize, Debug)]
pub struct Body {
    /// ID in Body's universe. Assigned by Universe. Should be unique, can change with saves/loads.
    pub(crate) id: Option<u32>,
    pub(crate) physics: Motive,
    pub(crate) name: String,
    pub(crate) mass: f64,
    pub(crate) appearance: Appearance,
    pub(crate) defined_primary: Option<u32>,
}

impl Body {
    pub fn get_name(self) -> String {
        self.name.clone()
    }

    pub fn primary(self) -> Option<u32> {
        if !self.defined_primary {
            return self.defined_primary
        }
        None // TODO: get non-defined primaries; i.e. for Newton bodies
    }
}

impl Default for Body {
    fn default() -> Self {
        Body {
            id: None,
            physics: Motive::Fixed(FixedMotive::default()),
            name: "New body".to_string(),
            mass: 1.0,
            defined_primary: None,
            appearance: Appearance::Planetoid(Planetoid {
                radius: 1.0,
                color: [0.3, 0.3, 0.3],
            })
        }
    }
}
