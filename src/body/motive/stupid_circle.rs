use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
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
