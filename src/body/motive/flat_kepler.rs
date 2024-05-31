use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct FlatKepler {
    pub(crate) semi_major_axis: f64,
    pub(crate) mean_anomaly_at_epoch: f64,
    pub(crate) eccentricity: f64,
    pub(crate) longitude_of_periapsis: f64,
}

impl Default for FlatKepler {
    fn default() -> Self {
        FlatKepler {
            semi_major_axis: 10.0,
            mean_anomaly_at_epoch: 0.0,
            eccentricity: 0.75,
            longitude_of_periapsis: 0.0,
        }
    }
}
