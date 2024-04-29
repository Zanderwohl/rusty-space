use std::collections::HashMap;
use std::ops::Add;
use glam::DVec3;
use bevy::prelude::Resource;
use crate::util::circular;

#[derive(Resource)]
pub struct Universe {
    bodies: HashMap<u32, NewBody>,
    counter: u32,
}

impl Default for Universe {
    fn default() -> Self {
        Universe {
            bodies: HashMap::new(),
            counter: 0,
        }
    }
}

impl Universe {
    pub fn new() -> Universe {
        Universe::default()
    }

    pub fn get_body(&self, id: u32) -> Option<&NewBody> {
        self.bodies.get(&id)
    }

    fn next_id(&mut self) -> u32 {
        let id = self.counter;
        self.counter += 1;
        id
    }

    pub fn add_body(mut self, body: NewBody) {
        let id = self.next_id();
        self.bodies.insert(id, body);
    }

    pub fn remove_body(mut self, id: u32) {
        self.bodies.remove(&id);
    }

    pub fn calc_positions_at_time(&self, time: f64) -> HashMap<u32, DVec3> {
        let mut positions: HashMap<u32, DVec3> = HashMap::new();
        for (&id, body) in self.bodies.iter() {
            let origin = self.calc_origin_at_time(time, body);
            let position = self.calc_position_at_time(time, body, origin);
            positions.insert(id, position);
        }
        positions
    }

    fn calc_origin_at_time(&self, time: f64, body: &NewBody) -> DVec3 {
        if body.parent.is_some() {
            let parent_id = body.parent.unwrap();
            let parent = self.bodies.get(&parent_id).unwrap(); // If we crash here, then parent IDs aren't getting inserted/updated/deleted properly
            let parent_origin = self.calc_origin_at_time(time, parent);
            self.calc_position_at_time(time, parent, parent_origin)
        } else {
            DVec3::ZERO
        }
    }

    fn calc_position_at_time(&self, time: f64, body: &NewBody, origin: DVec3) -> DVec3 {
        match &body.physics {
            Motive::Fixed(fixed_motive) => {
                origin + fixed_motive.local_position
            },
            Motive::Linear(linear_motive) => {
                origin + linear_motive.local_velocity * time
            },
            Motive::StupidCircle(circular_motive) => {
                let mu = 1.0;
                let v = circular::true_anomaly::at_time(time, circular_motive.radius, mu);
                let local_p = circular::position::from_true_anomaly(circular_motive.radius, v);
                origin + local_p
            },
            _ => { origin }
        }
    }
}

pub struct NewBody {
    physics: Motive,
    name: String,
    mass: f64,
    radius: f64,
    parent: Option<u32>,
}

impl Default for NewBody {
    fn default() -> Self {
        NewBody {
            physics: Motive::Fixed(FixedMotive::default()),
            name: "New body".to_string(),
            mass: 1.0,
            radius: 1.0,
            parent: None,
        }
    }
}

/// A Motive is a method by which a body can move.
enum Motive {
    Fixed(FixedMotive),
    Linear(LinearMotive),
    StupidCircle(StupidCircle),
}

struct FixedMotive {
    local_position: DVec3,
}

impl Default for FixedMotive {
    fn default() -> Self {
        FixedMotive {
            local_position: DVec3::ZERO,
        }
    }
}

struct LinearMotive {
    local_position: DVec3,
    local_velocity: DVec3,
}

impl Default for LinearMotive {
    fn default() -> Self {
        LinearMotive {
            local_position: DVec3::ZERO,
            local_velocity: DVec3::new(1.0, 0.0, 0.0),
        }
    }
}

struct StupidCircle {
    radius: f64,
}

impl Default for StupidCircle {
    fn default() -> Self {
        StupidCircle {
            radius: 1.0,
        }
    }
}
