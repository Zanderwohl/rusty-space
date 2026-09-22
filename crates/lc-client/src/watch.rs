//! The telescope, when no shard runs it: a test or a headless snapshot runs the same
//! `lc_world` code here. With a shard, this client only holds a copy of what its craft knows.
//! See `lightcone/docs/24-standing-instruments.md`.

use lc_world::knowledge::observatory::{self, Station};
use lc_world::knowledge::survey::Optics;

use crate::session::Session;

pub use lc_world::knowledge::observatory::CHARTS;

impl Session {
    /// One hull, so the baseline is the mirror. A formation of telescopes is
    /// [`Optics::joined`], which resolves more next to a bright source.
    pub fn optics(&self) -> Optics {
        Optics::of(self.telescope)
    }

    fn telescope_station(&self) -> Station {
        Station { position_ly: self.ship.motion.position_ly, instrument: self.telescope }
    }

    /// Only when no shard is running: instruments run here beside a shard would record the
    /// sky twice.
    pub fn tick_instruments(&mut self, integration_s: f64) {
        let now = self.coordinate_time_s();
        let at = self.telescope_station();
        self.observatory.integration_s = integration_s;
        self.observatory.tick(&mut self.sky_model, &mut self.knowledge, at, now);
        if let Some(id) = self.observatory.pointing() {
            self.aim(Some(id));
        }
    }

    /// Only when there is no shard to issue them.
    pub fn issue_charts(&mut self, reach_ly: f64) {
        let now = self.coordinate_time_s();
        let at = self.telescope_station();
        observatory::issue_charts(&mut self.sky_model, &mut self.knowledge, at, reach_ly, now);
    }
}
