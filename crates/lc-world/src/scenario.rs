//! Scenes to put in a world, so a ship has something to be near.
//!
//! Everything this game is about is a *relation* between two craft — a five-kilometre hull
//! standing off a five-hundred-metre one, an approach seen from the craft being approached, a
//! stern chase that takes three months — and none of it can be looked at with one ship in the
//! sky. What existed before this was a flag that parked a fan of unnamed hulls near the player
//! and left them there, which photographs a hull and nothing a hull does.
//!
//! **A scene is a schedule, not a cutscene.** Every beat below is the same
//! [`crate::motion::Change`] a player's order becomes, folded by the same function, arriving
//! through the same light-cone gate. Nothing here is animated and nothing here is faked: a
//! craft told to fly somewhere flies there, taking as long as it takes, and is seen when its
//! light arrives. The one thing a director may do that no client may is put a craft somewhere
//! to begin with, which is why the runner is server-side — see `lc_server::director`.
//!
//! Data only, and in this crate rather than the server's because both ends read it: the client
//! names the scenes on a panel, and a catalogue on the wire would be a second copy of a list.

use crate::craft::Kind;

/// Identifiers from here up, so a scene's cast cannot collide with a ship a sign-in minted
/// (which counts from one) or with anything else placed into a world.
pub const BASE_ID: i64 = 2_000;

/// Which craft in a scene something is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// The player's own craft, and the only one the camera can be behind: the eye is bound to
    /// the ship the session owns and there is nowhere else to put it.
    Pov,
    /// One of the cast, by position in [`Scenario::cast`].
    Cast(usize),
}

/// Where a craft is when the scene opens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Start {
    /// Left exactly where it already is.
    ///
    /// For the player, that is wherever signing in put them, which is open space a few AU out
    /// from the star. A scene that is only about hulls wants nothing else in frame — a planet
    /// behind them is a planet they are all silhouetted against.
    AsFound,
    /// Resolved against the system and then held — the sequence `--station` already uses to
    /// put the player on a station without flying them there.
    ///
    /// Spelled the way [`crate::navigation::Course::parse`] reads: `orbit:Jupiter:low`,
    /// `polar:Saturn:distant`, `belt:2`, `leave`.
    Holding(&'static str),
    /// On the same orbit as another craft, that many of its own hull lengths round it.
    ///
    /// A real co-orbit rather than a point in space, so it holds for ever: both craft are on
    /// the same circle at the same rate, a fixed arc apart. **Placed there rather than flown
    /// there**, and the difference matters — a planner asks a circle which side of it is
    /// nearest the ship coming in and rewrites the phase to suit, so a course can name an orbit
    /// but not a point on one. Standing somebody up beside somebody else is the authority's
    /// job, and it is the reason there is a director at all.
    Alongside { of: Slot, lengths: f64 },
    /// Off the player's shoulder, that many of *its own* hull lengths along a bearing in
    /// simulation axes.
    ///
    /// Range in hull lengths rather than metres because the sizes span two decades: sixteen
    /// lengths puts a five-hundred-metre hull and a fifty-kilometre one the same width on
    /// screen, which is the only arrangement where they are both worth looking at in one frame.
    ///
    /// Held where it is put. A world runs at thousands of times real time, so the slowest speed
    /// worth calling a speed carries a craft out of frame before a shutter opens.
    Beside { lengths: f64, bearing: [f64; 3] },
}

/// One craft in a scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Member {
    /// What it is called on screen. A craft without one is drawn as "Ship 2000", which is what
    /// the flag this replaces did and is not a name.
    pub name: &'static str,
    pub kind: Kind,
    /// Metres, within [`crate::craft::LENGTH_RANGE_M`] — the span the camera and the reticle
    /// are built for. It also sets how fast the hull comes about: a fifty-kilometre ship turns
    /// a hundred times slower than a five-hundred-metre one.
    pub length_m: f64,
    /// What it flies with, in g.
    ///
    /// The one lever that makes a chase a chase. [`Kind`] cannot do it — every `Kind::Ship` is
    /// the same five g — and `length_m` reaches only the slew rate and the mass.
    pub accel_g: f64,
    pub start: Start,
}

/// Something that happens to one craft, at one offset into the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beat {
    /// Coordinate seconds after the scene was staged. Not a wall clock: a beat is a point on a
    /// worldline, and a world that runs sixty times over reaches it sixty times sooner.
    pub after_s: f64,
    pub actor: Slot,
    pub act: Act,
}

/// What a beat does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    /// Fly a course, in [`crate::navigation::Course::parse`]'s spelling.
    Fly(&'static str),
    /// Close on another craft and hold station off it.
    ///
    /// The standing order, re-solved by the authority against sightings, exactly as a player's
    /// would be — and the only way to *meet* somebody. Being sent to the same orbit is not
    /// meeting them: two craft sent to `orbit:Jupiter:low` from different places each arrive at
    /// whatever point of the circle was nearest them, which can be opposite sides of the
    /// planet. The standoff it settles at is worked out from both hulls, so a five-kilometre
    /// ship stands further off than a five-hundred-metre one and the picture is the same.
    Chase(Slot),
    BreakOff,
    /// Cut the drive. Not a stop: whatever the ship was doing at the time, it keeps doing
    /// ballistically.
    Cut,
}

/// A scene: who is in it, where they start, and what they do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scenario {
    /// What `--demo` takes and what a client names on the wire.
    pub name: &'static str,
    /// One line, for the button.
    pub blurb: &'static str,
    /// The catalogue star to stage in. `"Sol"` is the one system with measured rather than
    /// generated bodies, which is the only place Jupiter and Saturn exist by name.
    pub star: &'static str,
    /// How fast the world runs while this is staged, as a multiple of the design rate of one
    /// Julian year an hour. Sixty is a year a minute.
    pub rate: f64,
    /// What the player's own craft does. Its hull stays whatever the session gave it: a
    /// welcome carries a motion, not a shipyard.
    pub pov: Member,
    pub cast: &'static [Member],
    pub beats: &'static [Beat],
}

impl Scenario {
    /// Every scene there is.
    pub const ALL: &'static [Scenario] = &[TRAFFIC, MEETING, APPROACH, CHASE];

    /// The one whose name starts with `prefix`, if exactly one does.
    ///
    /// A prefix rather than the whole name because these are typed at a shell, and ambiguity
    /// is refused rather than guessed at — `--demo c` naming two scenes should be an answer
    /// of "which", not whichever was declared first.
    pub fn named(prefix: &str) -> Option<&'static Scenario> {
        let lower = prefix.to_lowercase();
        let mut found = Self::ALL.iter().filter(|s| s.name.starts_with(&lower));
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// The craft in a slot, or `None` for a cast member that is not in this scene.
    pub fn member(&self, slot: Slot) -> Option<&Member> {
        match slot {
            Slot::Pov => Some(&self.pov),
            Slot::Cast(at) => self.cast.get(at),
        }
    }
}

/// Company, and nothing else. What `--traffic` was: craft to be photographed beside, spread
/// across the designed range of sizes so the reticle and the camera can be judged against both
/// ends of it at once.
///
/// Geometric across the range rather than linear, because the range is two decades and
/// stepping it evenly would make every one of them large but the first.
pub const TRAFFIC: Scenario = Scenario {
    name: "traffic",
    blurb: "Four hulls to stand beside, from five hundred metres to fifty kilometres.",
    star: "Sol",
    rate: 1.0,
    pov: Member {
        name: "Kestrel",
        kind: Kind::Ship,
        length_m: 500.0,
        accel_g: 5.0,
        start: Start::AsFound,
    },
    cast: &[
        Member {
            name: "Wren",
            kind: Kind::Ship,
            length_m: 500.0,
            accel_g: 5.0,
            start: Start::Beside { lengths: 16.0, bearing: [1.0, 0.28, 0.0] },
        },
        Member {
            name: "Harrier",
            kind: Kind::Ship,
            length_m: 2_200.0,
            accel_g: 5.0,
            start: Start::Beside { lengths: 16.0, bearing: [1.0, 0.0, 0.28] },
        },
        Member {
            name: "Bittern",
            kind: Kind::Ship,
            length_m: 10_600.0,
            accel_g: 5.0,
            start: Start::Beside { lengths: 16.0, bearing: [1.0, -0.28, 0.0] },
        },
        Member {
            name: "Albatross",
            kind: Kind::Ship,
            length_m: 50_000.0,
            accel_g: 5.0,
            start: Start::Beside { lengths: 16.0, bearing: [1.0, 0.0, -0.28] },
        },
    ],
    beats: &[],
};

/// A big ship comes down to a small one, in low orbit of the biggest thing there is.
///
/// The scale demonstration. Ten hull lengths apart, a five-kilometre ship from a
/// five-hundred-metre one is a building seen from a car, and Jupiter behind it is neither.
pub const MEETING: Scenario = Scenario {
    name: "meeting",
    blurb: "A five-kilometre ship comes down to meet you, in low orbit of Jupiter.",
    star: "Sol",
    rate: 1.0,
    pov: Member {
        name: "Kestrel",
        kind: Kind::Ship,
        length_m: 500.0,
        accel_g: 5.0,
        start: Start::Holding("orbit:Jupiter:low"),
    },
    cast: &[Member {
        name: "Anvil",
        kind: Kind::Ship,
        length_m: 5_000.0,
        accel_g: 5.0,
        start: Start::Alongside { of: Slot::Pov, lengths: 12.0 },
    }],
    // Nothing happens, and that is the scene. Two ships holding the same orbit a fixed arc
    // apart, with a planet filling the window behind them: what a chase *ends* at, without the
    // quarter of an orbit a pursuit curve spends getting there.
    beats: &[],
};

/// Being approached, which is not the same picture as approaching.
///
/// Over the pole, because that is the only place Saturn's rings are edge-on and the only orbit
/// from which all of a body goes past underneath.
pub const APPROACH: Scenario = Scenario {
    name: "approach",
    blurb: "You hold a polar orbit of Saturn. Something much larger closes on you.",
    star: "Sol",
    rate: 1.0,
    pov: Member {
        name: "Kestrel",
        kind: Kind::Ship,
        length_m: 500.0,
        accel_g: 5.0,
        start: Start::Holding("polar:Saturn:low"),
    },
    cast: &[Member {
        name: "Anvil",
        kind: Kind::Ship,
        length_m: 5_000.0,
        // A pursuit curve against a craft in orbit spends the approach chasing where the
        // quarry was, so the approach has to be short against the period it is chasing round:
        // a hundred thousand kilometres at five g is a sixth of an orbit, which converges.
        accel_g: 5.0,
        start: Start::Holding("polar:Saturn:high"),
    }],
    beats: &[Beat {
        after_s: 0.0,
        actor: Slot::Cast(0),
        act: Act::Chase(Slot::Pov),
    }],
};

/// Three months of running, in fifteen seconds of watching.
///
/// The quarry leaves for the Oort cloud — population index two, the outermost of the three a
/// system is generated with — and is followed by something that pulls twice as hard. The
/// chase is ordered while both are still near the primary, because an intercept is refused
/// against a craft whose light has not arrived, and out there it has hours to travel.
///
/// The distance is the point: it stays inside the shell that decides which system a craft is
/// in, so the two never lose sight of each other, and it is far enough that a flip-and-burn
/// over it is a hundred and fifty days.
pub const CHASE: Scenario = Scenario {
    name: "chase",
    blurb: "A quarry runs for the Oort cloud. You pull twice as hard. Three months, watched at a year a minute.",
    star: "Sol",
    rate: 60.0,
    pov: Member {
        name: "Kestrel",
        kind: Kind::Ship,
        length_m: 500.0,
        accel_g: 10.0,
        start: Start::Holding("orbit:Jupiter:high"),
    },
    cast: &[Member {
        name: "Quarry",
        kind: Kind::Ship,
        length_m: 500.0,
        accel_g: 5.0,
        start: Start::Beside { lengths: 40.0, bearing: [1.0, 0.1, 0.0] },
    }],
    beats: &[
        Beat { after_s: 0.0, actor: Slot::Cast(0), act: Act::Fly("belt:2") },
        // Late enough that the quarry is plainly running, early enough that its light is still
        // minutes rather than hours away.
        Beat { after_s: 3_600.0, actor: Slot::Pov, act: Act::Chase(Slot::Cast(0)) },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::Course;
    use crate::system::LocalSystem;

    /// Every spelling in the catalogue is one the parser reads. A typo here is a scene that
    /// stages a craft nowhere, and the failure would be a missing ship rather than an error.
    #[test]
    fn every_course_in_every_scene_parses() {
        for scene in Scenario::ALL {
            let starts = std::iter::once(&scene.pov)
                .chain(scene.cast)
                .filter_map(|m| match m.start {
                    Start::Holding(spelling) => Some(spelling),
                    Start::AsFound
                    | Start::Alongside { .. }
                    | Start::Beside { .. } => None,
                });
            let flown = scene.beats.iter().filter_map(|b| match b.act {
                Act::Fly(spelling) => Some(spelling),
                _ => None,
            });
            for spelling in starts.chain(flown) {
                assert!(
                    Course::parse(spelling).is_some(),
                    "{}: {spelling:?} is not a course",
                    scene.name,
                );
            }
        }
    }

    /// A beat about a craft that is not in the scene is a beat that silently does nothing.
    #[test]
    fn every_beat_is_about_somebody_who_is_there() {
        for scene in Scenario::ALL {
            let slots = scene.beats.iter().flat_map(|beat| match beat.act {
                Act::Chase(on) => vec![beat.actor, on],
                _ => vec![beat.actor],
            });
            for slot in slots {
                assert!(scene.member(slot).is_some(), "{}: nobody in {slot:?}", scene.name);
            }
        }
        // And the same for a start measured against somebody else. A craft placed alongside
        // nobody is a craft left at the origin, which is empty interstellar space.
        for scene in Scenario::ALL {
            for member in std::iter::once(&scene.pov).chain(scene.cast) {
                let Start::Alongside { of, .. } = member.start else { continue };
                assert!(
                    scene.member(of).is_some(),
                    "{}: {} stands beside nobody",
                    scene.name,
                    member.name,
                );
                assert!(of != Slot::Pov || !std::ptr::eq(member, &scene.pov),
                    "{}: {} stands beside itself", scene.name, member.name);
            }
        }
    }

    /// Beats are read in order and fired once. Out of order, a scene would skip whatever came
    /// before the beat that overtook it.
    #[test]
    fn beats_are_in_the_order_they_happen() {
        for scene in Scenario::ALL {
            let times: Vec<f64> = scene.beats.iter().map(|b| b.after_s).collect();
            assert!(
                times.windows(2).all(|pair| pair[0] <= pair[1]),
                "{}: {times:?} is not in order",
                scene.name,
            );
            assert!(times.iter().all(|t| *t >= 0.0), "{}: a beat before the scene", scene.name);
        }
    }

    /// Hulls stay inside the range the camera and the reticle are built for, and nothing flies
    /// with an acceleration nobody could survive or that would not move.
    #[test]
    fn every_hull_is_one_the_camera_is_built_for() {
        let (smallest, largest) = crate::craft::LENGTH_RANGE_M;
        for scene in Scenario::ALL {
            for member in std::iter::once(&scene.pov).chain(scene.cast) {
                assert!(
                    (smallest..=largest).contains(&member.length_m),
                    "{}: {} is {} m",
                    scene.name,
                    member.name,
                    member.length_m,
                );
                assert!(member.accel_g > 0.0, "{}: {} cannot move", scene.name, member.name);
            }
        }
    }

    /// Names are how a scene is asked for, so no two may be confusable and none may be a
    /// prefix of another — `--demo c` must mean one thing or refuse.
    #[test]
    fn no_scene_shadows_another() {
        for scene in Scenario::ALL {
            assert_eq!(Scenario::named(scene.name).map(|s| s.name), Some(scene.name));
            assert_eq!(scene.name, scene.name.to_lowercase(), "a name typed at a shell");
        }
        assert!(Scenario::named("nonesuch").is_none());
        // Ambiguity refuses rather than picking the first declared.
        let shared = Scenario::ALL.iter().filter(|s| s.name.starts_with('c')).count();
        if shared > 1 {
            assert!(Scenario::named("c").is_none(), "an ambiguous prefix resolved");
        }
    }

    /// **The test that catches a misspelt moon.** A course that parses is not a course that
    /// exists: `orbit:Juipter:low` reads perfectly and resolves to nothing, and the symptom
    /// would be a craft that never appears rather than an error anybody sees.
    ///
    /// Skips when the catalogue is not on disk, as the other tests that need a real sky do.
    #[cfg(feature = "hyg")]
    #[test]
    fn every_course_resolves_against_the_system_it_is_staged_in() {
        use crate::sky::{CatalogueStar, StarProvider};
        let Ok(provider) = crate::sky::hyg::HygProvider::load(
            "../../assets/catalogs/hygdata_v42_dist_sort.csv",
        ) else {
            return;
        };
        for scene in Scenario::ALL {
            let star: CatalogueStar = provider
                .stars()
                .iter()
                .find(|s| s.name.as_deref() == Some(scene.star))
                .unwrap_or_else(|| panic!("{}: no star called {}", scene.name, scene.star))
                .clone();
            let system = LocalSystem::for_star(&star)
                .unwrap_or_else(|| panic!("{}: {} has no system", scene.name, scene.star));
            let here = system.origin_ly;

            let starts = std::iter::once(&scene.pov).chain(scene.cast).filter_map(|m| {
                match m.start {
                    Start::Holding(spelling) => Some(spelling),
                    Start::AsFound
                    | Start::Alongside { .. }
                    | Start::Beside { .. } => None,
                }
            });
            let flown = scene.beats.iter().filter_map(|b| match b.act {
                Act::Fly(spelling) => Some(spelling),
                _ => None,
            });
            for spelling in starts.chain(flown) {
                let course = Course::parse(spelling).expect("parsed by the test above");
                assert!(
                    course.resolve(&system, here, 0.0).is_some(),
                    "{}: {spelling:?} is nowhere in {}",
                    scene.name,
                    scene.star,
                );
            }
        }
    }

    /// The chase is the one scene whose whole point is a difference in acceleration.
    #[test]
    fn the_chase_is_a_chase() {
        assert!(CHASE.pov.accel_g > CHASE.cast[0].accel_g, "the quarry is not being caught");
        assert!(CHASE.rate > 1.0, "three months at the design rate is a quarter of an hour");
    }
}
