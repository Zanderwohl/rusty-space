//! The telescope, when this client runs it itself.
//!
//! With a shard, it does not: the shard runs every craft's instruments whether or not anybody is
//! signed in, and this client holds a copy of what its craft knows. Without one — a test, a
//! headless snapshot — the same `lc_world` code runs here instead. See
//! `lightcone/docs/24-standing-instruments.md`.

use lc_world::knowledge::observatory::{self, Station};
use lc_world::knowledge::survey::Optics;

use crate::session::Session;

pub use lc_world::knowledge::observatory::CHARTS;

impl Session {
    /// What the ship looks through, and how far apart its elements are.
    ///
    /// One hull, so the baseline is the mirror. A swarm of telescopes flying in formation is
    /// [`Optics::joined`], and the difference is what it can see next to something bright.
    pub fn optics(&self) -> Optics {
        Optics::of(self.telescope)
    }

    fn telescope_station(&self) -> Station {
        Station { position_ly: self.ship.motion.position_ly, instrument: self.telescope }
    }

    /// Run whatever the telescope is committed to, for however much coordinate time has passed.
    ///
    /// Only when no shard is running it: a client that ran its craft's instruments beside a
    /// shard would be two telescopes recording one sky.
    pub fn tick_instruments(&mut self, integration_s: f64) {
        let now = self.coordinate_time_s();
        let at = self.telescope_station();
        self.observatory.integration_s = integration_s;
        self.observatory.tick(&mut self.sky_model, &mut self.knowledge, at, now);
        if let Some(id) = self.observatory.pointing() {
            self.aim(Some(id));
        }
    }

    /// Issue the charts a ship leaves port with, when there is no shard to issue them.
    pub fn issue_charts(&mut self, reach_ly: f64) {
        let now = self.coordinate_time_s();
        let at = self.telescope_station();
        observatory::issue_charts(&mut self.sky_model, &mut self.knowledge, at, reach_ly, now);
    }
}
