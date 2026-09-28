//! Every open face an emission leaves through, lit at its share of what leaves its end, whatever
//! lit it: the main drive aft at `F c`, an emit fore, aft or both. 32 §The exhaust cone.
//!
//! The player's own from what its craft has lit now, and another craft's from what its `Presence`
//! stated of each end beside `drive_w`, as the light left it. Worked out once a frame, before the
//! hulls, whose emitter grid it lights, and the exhaust, whose aperture glow it places. A distant
//! craft's point reads the same.

use std::collections::HashMap;

use bevy::prelude::*;
use lc_proto::ShipId;
use lc_world::emit::Ends;
use lc_world::fitting::Balance;
use lc_world::form::Form;
use lc_world::form::capacity::{Aperture, apertures};

use crate::session::Session;
use crate::ship_hull::RealHulls;
use crate::uplink::{Contact, Uplink};

/// One open face this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub aperture: Aperture,
    /// What leaves through it, W.
    pub power_w: f64,
    /// A blackbody radiating all of it through its area, K. Zero when dark.
    pub temperature_k: f64,
}

/// A craft's faces this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lit {
    pub ends: Ends,
    /// Every engine's face in [`apertures`]' order, which is the index its hull's mesh names.
    pub faces: Vec<Face>,
}

impl Lit {
    fn of(ends: Ends, apertures: &[Aperture]) -> Self {
        let faces = apertures
            .iter()
            .map(|aperture| Face { aperture: *aperture, power_w: ends.through(aperture), temperature_k: ends.face_k(aperture) })
            .collect();
        Self { ends, faces }
    }
}

#[derive(Resource, Default)]
pub struct LitFaces {
    /// Each craft's faces, by the hash of the form they were worked out from.
    apertures: HashMap<Option<ShipId>, (u64, Vec<Aperture>)>,
    lit: HashMap<Option<ShipId>, Lit>,
}

impl LitFaces {
    /// `None` for the player's own. Nothing for a craft with no form drawn, or none lit.
    pub fn of(&self, craft: Option<ShipId>) -> Option<&Lit> {
        self.lit.get(&craft)
    }

    pub fn iter(&self) -> impl Iterator<Item = (Option<ShipId>, &Lit)> {
        self.lit.iter().map(|(craft, lit)| (*craft, lit))
    }
}

/// What another craft's faces send, as its `Presence` stated.
pub fn stated(contact: &Contact) -> Ends {
    contact.emit.with_drive(contact.drive_w)
}

/// What the player's own faces send now.
pub fn own(session: &Session, balance: &Balance) -> Ends {
    let ship = &session.ship;
    lc_world::emit::faces_w(ship, balance, session.coordinate_time_s())
}

/// What a face at `kelvin` sends through this observer's bands, on the exposure's scale: the
/// units of a hull's `reflected`. Nothing for a dark face.
pub fn radiance(session: &Session, kelvin: f64) -> glam::DVec3 {
    if kelvin <= 0.0 {
        return glam::DVec3::ZERO;
    }
    Vec3::from_array(session.mapping.apply(&crate::session::spectrum_at(kelvin))).as_dvec3()
}

/// Before the hulls: a form's hash is the one [`RealHulls`] last stated for its craft.
pub fn light_faces(
    game: Res<crate::app::Game>,
    uplink: Res<Uplink>,
    own_form: Res<crate::parts::OwnForm>,
    real: Res<RealHulls>,
    mut faces: ResMut<LitFaces>,
) {
    let session = &game.0;
    let balance = uplink.fitting.as_ref().map_or(Balance::DEFAULT, |f| f.balance.into());
    let own_ends = own(session, &own_form.balance());
    let crafts = std::iter::once((None, own_ends)).chain(uplink.contacts.iter().map(|c| (Some(c.ship_id), stated(c))));
    let faces = &mut *faces;
    faces.lit.clear();
    faces.apertures.retain(|craft, _| real.stated(*craft).is_some());
    for (craft, ends) in crafts {
        if ends.is_dark() {
            continue;
        }
        let Some(hash) = real.stated(craft) else { continue };
        let form = || match craft {
            None => own_form.form().map(|f| (f.clone(), own_form.balance())),
            Some(id) => uplink
                .contacts
                .iter()
                .find(|c| c.ship_id == id)
                .filter(|c| !c.form.parts.is_empty())
                .map(|c| (Form::from(&c.form), balance)),
        };
        if faces.apertures.get(&craft).is_none_or(|(held, _)| *held != hash) {
            let Some((form, balance)) = form() else { continue };
            faces.apertures.insert(craft, (hash, apertures(&form, &balance).unwrap_or_default()));
        }
        let Some((_, apertures)) = faces.apertures.get(&craft) else { continue };
        faces.lit.insert(craft, Lit::of(ends, apertures));
    }
}

#[cfg(test)]
mod tests {
    use lc_world::emit::aperture_temperature_k;
    use lc_world::form::presets::Builtin;

    use super::*;

    const B: Balance = Balance::DEFAULT;

    /// Every face's power and temperature on [`two_ended`] for `ends`.
    fn lit(ends: Ends) -> Lit {
        let faces = apertures(&two_ended(), &B).unwrap();
        assert!(faces.iter().any(Aperture::fore) && faces.iter().any(Aperture::aft));
        Lit::of(ends, &faces)
    }

    /// The plate with one of its pair turned to fire fore.
    fn two_ended() -> Form {
        lc_world::form::presets::turned_fore(Builtin::Plate.form(), 1)
    }

    /// Only the ends an emission leaves through are lit, each face at its share of its end.
    #[test]
    fn only_the_ends_it_uses_are_lit() {
        let faces = apertures(&two_ended(), &B).unwrap();
        let aft_w = 4.0e19;
        let drive = Lit::of(Ends::default().with_drive(aft_w), &faces);
        for face in &drive.faces {
            let want = if face.aperture.aft() { aft_w * face.aperture.share } else { 0.0 };
            assert_eq!(face.power_w, want);
            assert_eq!(face.temperature_k, aperture_temperature_k(want, face.aperture.area_m2()));
        }
        let aft: f64 = drive.faces.iter().map(|f| f.power_w).sum();
        assert!((aft / aft_w - 1.0).abs() < 1.0e-12, "the aft faces carry all of it: {aft}");
    }

    /// 32's figure: the starting drive at its rating, through its bell's face, 176 m across.
    #[test]
    fn the_starting_face_is_five_hundred_thousand_kelvin() {
        let start = Form::starting();
        let rating = lc_world::form::capacity::aft_aperture_w(&start, &B).unwrap();
        let lit = Lit::of(Ends::default().with_drive(rating), &apertures(&start, &B).unwrap());
        let k = lit.faces[0].temperature_k;
        assert!((k / 5.3e5 - 1.0).abs() < 0.01, "{k} K");
    }

    /// A fore emit lights the bow and not the stern; nothing lit lights nothing.
    #[test]
    fn a_fore_emit_lights_the_bow_and_nothing_lights_nothing() {
        let fore = lit(Ends { fore_w: 2.0e19, aft_w: 0.0 });
        let dark = lit(Ends::default());
        for (face, unlit) in fore.faces.iter().zip(&dark.faces) {
            assert_eq!(face.power_w > 0.0, face.aperture.fore(), "{face:?}");
            assert_eq!((unlit.power_w, unlit.temperature_k), (0.0, 0.0));
        }
    }

    /// A balanced emit lights both ends, each at what it sends.
    #[test]
    fn a_balanced_emit_lights_both_ends() {
        let per_end = 1.5e19;
        let balanced = lit(Ends { fore_w: per_end, aft_w: per_end });
        let end = |fore: bool| balanced.faces.iter().filter(|f| f.aperture.fore() == fore).map(|f| f.power_w).sum::<f64>();
        assert!((end(true) / per_end - 1.0).abs() < 1.0e-12 && (end(false) / per_end - 1.0).abs() < 1.0e-12);
    }
}
