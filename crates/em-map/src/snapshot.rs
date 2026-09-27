//! What there is to draw, and when it was true.

use glam::DVec3;

/// Meters in a light-year. The crate's interface is light-years and meters, so it names the
/// conversion between them; `lc_world::system::M_PER_LY` is the same number for the same reason.
pub const M_PER_LY: f64 = 9.460_730_472_580_8e15;

/// Meters in an astronomical unit, by definition.
pub const M_PER_AU: f64 = 1.495_978_707e11;

/// How a whole snapshot was arrived at.
///
/// Per snapshot, not per item: a picture is one claim about what is where at one moment, and a
/// mixture with no label on the join leaves the reader to audit which half is which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    /// One observer's instruments. Everything is where its light says it was.
    Observed,
    /// Several sources folded together. Nobody ever saw this from one place at one time.
    Aggregate,
    /// Read off the coordinate clock, with no light delay. Development and replays.
    Coordinate,
}

/// What a thing is, which decides how it is drawn and what color it takes. The color itself
/// is the host's: a palette belongs to a product.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Star,
    Planet,
    Moon,
    /// Anything smaller that is still its own body: an asteroid, a comet.
    Minor,
    Station,
    Ship,
    /// A belt, a ring system, a cloud. Drawn from [`MapItem::annulus_m`] rather than a radius.
    Population,
    /// Whoever the snapshot was taken by.
    Observer,
}

/// Stable across frames, so an item that has not moved keeps its entity and its selection.
///
/// A provider makes one from whatever it already has that is stable — a ship's id, a hash of a
/// body's name. Not an index: a list that reorders would silently retarget every selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemKey(pub u64);

impl ItemKey {
    /// FNV-1a of a name, matching `em_sim::BodyId`'s construction so two things named alike
    /// key alike.
    pub const fn from_name(name: &str) -> Self {
        Self(fnv(0xcbf2_9ce4_8422_2325, name.as_bytes()))
    }

    /// A key for an id that is only unique inside its own domain.
    ///
    /// A star's catalog id and a ship's id are small integers counted from different places,
    /// so raw they collide, and a collision is two things sharing one entity and one
    /// selection. The domain is hashed in first.
    pub const fn from_id(domain: &str, id: u64) -> Self {
        Self(fnv(fnv(0xcbf2_9ce4_8422_2325, domain.as_bytes()), &id.to_le_bytes()))
    }
}

const fn fnv(mut hash: u64, bytes: &[u8]) -> u64 {
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    hash
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapItem {
    pub key: ItemKey,
    pub label: String,
    pub kind: ItemKind,
    /// Light-years from the world origin, simulation axes.
    pub position_ly: DVec3,
    /// Meters. Zero for anything with no size worth drawing at any zoom.
    pub radius_m: f64,
    /// What wins when two labels want the same pixels, and how big a mark is drawn.
    /// Kilograms for a body. Zero means unstated, which is named only where there is room to
    /// spare and drawn at half size. See [`crate::weight`].
    pub weight: f64,
    /// Spin axis, or the normal of a ring or belt. Ecliptic north where nothing says otherwise.
    pub pole: DVec3,
    /// How far it reaches, for something shaped like a ring or a shell rather than a ball.
    ///
    /// Meters, and it carries the half-angle as well as the two radii: without it a cloud and
    /// a belt are the same pair of numbers, and one of them is a shell.
    pub annulus_m: Option<crate::outline::Extent>,
    /// Where else it might be, light-years from the world origin, for a position known only to
    /// an error. Each piece is one direction of it: a star placed by parallax is uncertain along
    /// the line of sight far more than across it, and the map should show which way.
    pub spread_ly: Vec<Spread>,
}

/// One direction of where something might be.
#[derive(Clone, Debug, PartialEq)]
pub enum Spread<P = DVec3> {
    /// A straight error bar between its two ends.
    Bar(P, P),
    /// An error along a curve, such as a phase along an orbit, where a chord would cut inside
    /// the path the thing is actually on. `closed` is a whole loop, and has no ends to cap.
    Arc { points: Vec<P>, closed: bool },
}

impl<P: Copy> Spread<P> {
    pub fn map<Q>(&self, f: impl Fn(P) -> Q) -> Spread<Q> {
        match self {
            Spread::Bar(near, far) => Spread::Bar(f(*near), f(*far)),
            Spread::Arc { points, closed } => Spread::Arc { points: points.iter().map(|p| f(*p)).collect(), closed: *closed },
        }
    }

    /// Each open end, with the point beside it that a cap is squared against.
    pub fn ends(&self) -> Vec<(P, P)> {
        match self {
            Spread::Bar(near, far) => vec![(*near, *far), (*far, *near)],
            Spread::Arc { closed: true, .. } => Vec::new(),
            Spread::Arc { points, .. } if points.len() >= 2 => {
                let n = points.len();
                vec![(points[0], points[1]), (points[n - 1], points[n - 2])]
            }
            Spread::Arc { .. } => Vec::new(),
        }
    }
}

impl MapItem {
    /// A body: something with a size and a pole.
    pub fn body(key: ItemKey, label: impl Into<String>, kind: ItemKind, position_ly: DVec3,
        radius_m: f64, pole: DVec3) -> Self {
        Self {
            key,
            label: label.into(),
            kind,
            position_ly,
            radius_m,
            weight: 0.0,
            pole,
            annulus_m: None,
            spread_ly: Vec::new(),
        }
    }

    /// Say that it is somewhere between `near_ly` and `far_ly`, not exactly where it is drawn.
    pub fn spread(mut self, near_ly: DVec3, far_ly: DVec3) -> Self {
        self.spread_ly.push(Spread::Bar(near_ly, far_ly));
        self
    }

    /// Say that it is somewhere along a curve through `points_ly`.
    pub fn spread_along(mut self, points_ly: Vec<DVec3>, closed: bool) -> Self {
        self.spread_ly.push(Spread::Arc { points: points_ly, closed });
        self
    }

    /// How much this one is worth naming, against everything else wanting the same pixels.
    /// Kilograms, where there is a mass to state.
    pub fn weighing(mut self, kg: f64) -> Self {
        self.weight = kg;
        self
    }

    /// A belt, a cloud or a ring system, about `position_ly` and in the plane of `pole`.
    pub fn annulus(key: ItemKey, label: impl Into<String>, position_ly: DVec3, pole: DVec3,
        extent: crate::outline::Extent) -> Self {
        Self {
            key,
            label: label.into(),
            kind: ItemKind::Population,
            position_ly,
            radius_m: 0.0,
            weight: 0.0,
            pole,
            annulus_m: Some(crate::outline::Extent {
                inner: extent.inner.min(extent.outer),
                outer: extent.inner.max(extent.outer),
                ..extent
            }),
            spread_ly: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapSnapshot {
    pub items: Vec<MapItem>,
    /// Coordinate seconds since J2000 this snapshot is stated at.
    pub epoch_s: f64,
    pub provenance: Provenance,
}

impl MapSnapshot {
    pub fn new(epoch_s: f64, provenance: Provenance, items: Vec<MapItem>) -> Self {
        Self { items, epoch_s, provenance }
    }

    pub fn observed(epoch_s: f64, items: Vec<MapItem>) -> Self {
        Self::new(epoch_s, Provenance::Observed, items)
    }

    pub fn coordinate(epoch_s: f64, items: Vec<MapItem>) -> Self {
        Self::new(epoch_s, Provenance::Coordinate, items)
    }

    pub fn item(&self, key: ItemKey) -> Option<&MapItem> {
        self.items.iter().find(|i| i.key == key)
    }

    pub fn observer(&self) -> Option<&MapItem> {
        self.items.iter().find(|i| i.kind == ItemKind::Observer)
    }

    /// How far the nearest and furthest items sit from `focus_ly`, meters.
    ///
    /// What ring selection is given. The nearest is often zero — the focus is usually *on*
    /// something — which is why [`crate::rings::decades`] has to survive a zero lower bound.
    pub fn extent_m(&self, focus_ly: DVec3) -> (f64, f64) {
        let mut near = f64::INFINITY;
        let mut far: f64 = 0.0;
        for item in &self.items {
            let d = item.position_ly.distance(focus_ly) * M_PER_LY;
            // An annulus is drawn out to its own edge, so it sets the extent even when its
            // center is the focus. Without this a system framed on its star is framed on
            // nothing, because every belt's center is the star.
            let outer = d + item.annulus_m.map_or(item.radius_m, |a| a.outer);
            near = near.min(d);
            far = far.max(outer);
        }
        if !near.is_finite() { near = 0.0 }
        (near, far)
    }

    /// Fold several snapshots into one, newest wins.
    ///
    /// The result is [`Provenance::Aggregate`] whatever went in, because it is: no observer
    /// saw it. Two sources naming the same key are the same thing seen twice, and the one
    /// stated later is the better answer.
    pub fn merge(parts: &[MapSnapshot]) -> MapSnapshot {
        let epoch_s = parts.iter().map(|p| p.epoch_s).fold(f64::NEG_INFINITY, f64::max);
        let mut items: Vec<MapItem> = Vec::new();
        let mut epochs: Vec<f64> = Vec::new();
        for part in parts {
            for item in &part.items {
                match items.iter().position(|held| held.key == item.key) {
                    Some(i) if epochs[i] >= part.epoch_s => {}
                    Some(i) => {
                        items[i] = item.clone();
                        epochs[i] = part.epoch_s;
                    }
                    None => {
                        items.push(item.clone());
                        epochs.push(part.epoch_s);
                    }
                }
            }
        }
        MapSnapshot::new(if epoch_s.is_finite() { epoch_s } else { 0.0 },
            Provenance::Aggregate, items)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn item(key: u64, at: DVec3) -> MapItem {
        MapItem::body(ItemKey(key), format!("body {key}"), ItemKind::Planet, at, 6.4e6, DVec3::Z)
    }

    #[test]
    fn merging_keeps_the_newest_statement_of_a_thing() {
        let old = MapSnapshot::observed(100.0, vec![item(7, DVec3::X)]);
        let new = MapSnapshot::observed(200.0, vec![item(7, DVec3::Y)]);

        // Either order, because a fold that depended on the argument order would be a fold
        // that gave two answers.
        for parts in [vec![old.clone(), new.clone()], vec![new.clone(), old.clone()]] {
            let merged = MapSnapshot::merge(&parts);
            assert_eq!(merged.items.len(), 1, "one thing seen twice is one thing");
            assert_eq!(merged.items[0].position_ly, DVec3::Y, "the older statement won");
            assert_eq!(merged.epoch_s, 200.0);
            assert_eq!(merged.provenance, Provenance::Aggregate);
        }
    }

    #[test]
    fn merging_is_a_union_over_keys() {
        let a = MapSnapshot::observed(1.0, vec![item(1, DVec3::X), item(2, DVec3::Y)]);
        let b = MapSnapshot::observed(1.0, vec![item(2, DVec3::Y), item(3, DVec3::Z)]);
        let merged = MapSnapshot::merge(&[a, b]);
        let mut keys: Vec<u64> = merged.items.iter().map(|i| i.key.0).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec![1, 2, 3]);
    }

    /// Nothing at all is a snapshot too — a ship between systems has one.
    #[test]
    fn an_empty_merge_is_not_infinite() {
        let merged = MapSnapshot::merge(&[]);
        assert!(merged.items.is_empty());
        assert_eq!(merged.epoch_s, 0.0);
        let (near, far) = merged.extent_m(DVec3::ZERO);
        assert!(near.is_finite() && far.is_finite(), "{near} {far}");
    }

    /// A belt centered on the focus still has an extent, because it is drawn out to its edge.
    /// Without that, framing a system on its star frames it on nothing.
    #[test]
    fn an_annulus_sets_the_extent_from_its_edge() {
        let belt = MapItem::annulus(ItemKey(1), "belt", DVec3::ZERO, DVec3::Z,
            crate::outline::Extent { inner: 3.0e11, outer: 5.0e11, half_angle_rad: 0.2 });
        let snapshot = MapSnapshot::observed(0.0, vec![belt]);
        let (near, far) = snapshot.extent_m(DVec3::ZERO);
        assert_eq!(near, 0.0);
        assert_eq!(far, 5.0e11);
    }

    /// The digest, not merely self-consistency.
    ///
    /// `em_sim::BodyId::from_name` is FNV-1a with the same basis and prime, so a body and its
    /// map item key alike without either crate importing the other — and pinning the value is
    /// what makes that a fact rather than a hope. A test that only compared this function to
    /// itself would pass with any hash at all.
    #[test]
    fn a_key_is_the_digest_a_body_id_is() {
        assert_eq!(ItemKey::from_name("Saturn"), ItemKey(0x2600_67d6_a62b_3a76));
        assert_eq!(ItemKey::from_name("Titan"), ItemKey(0x9a1c_f7d2_7e55_c2f9));
        assert_eq!(ItemKey::from_name(""), ItemKey(0xcbf2_9ce4_8422_2325), "the bare basis");
    }

    /// Two small integers counted from different places must not land on one key. Used raw
    /// they would collide constantly, and a collision is a star and a ship sharing one entity.
    #[test]
    fn a_domain_keeps_two_id_spaces_apart() {
        for id in 0..64u64 {
            assert_ne!(ItemKey::from_id("star", id), ItemKey::from_id("ship", id));
            assert_ne!(ItemKey::from_id("star", id), ItemKey::from_id("star", id + 1));
        }
        assert_eq!(ItemKey::from_id("star", 7), ItemKey::from_id("star", 7), "not stable");
    }
}
