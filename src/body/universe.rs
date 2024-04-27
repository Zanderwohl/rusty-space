use std::collections::HashMap;
use bevy::math::DVec3;
use bevy::prelude::{Has, Resource};

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
            let parent = self.bodies.get(&parent_id).unwrap();
            self.calc_origin_at_time(time, parent)
        } else {
            DVec3::ZERO
        }
    }

    fn calc_position_at_time(&self, time: f64, body: &NewBody, origin: DVec3) -> DVec3 {

    }
}

pub struct NewBody {
    physics: Box<dyn Motive + Send + Sync>,
    name: String,
    mass: f64,
    radius: f64,
    parent: Option<u32>,
}

impl Default for NewBody {
    fn default() -> Self {
        NewBody {
            physics: Box::new(FixedMotive::default()),
            name: "New body".to_string(),
            mass: 1.0,
            radius: 1.0,
            parent: None,
        }
    }
}

/// A Motive is a method by which a body can move.
pub trait Motive: Send + Sync {
    fn global_position_at_time(&self, time: f64) -> DVec3;
}

pub struct FixedMotive {
    pub position: DVec3,
}

impl Default for FixedMotive {
    fn default() -> Self {
        FixedMotive {
            position: DVec3::ZERO,
        }
    }
}

impl Motive for FixedMotive {
    fn global_position_at_time(&self, time: f64) -> DVec3 {
        self.position
    }
}

pub struct CircularMotive {
    radius: f64,
}

impl Motive for CircularMotive {
    fn global_position_at_time(&self, time: f64) -> DVec3 {
        // let parent_global_coords =
        // circular::true_anomaly::at_time(time, self.radius, )
        todo!()
    }
}

