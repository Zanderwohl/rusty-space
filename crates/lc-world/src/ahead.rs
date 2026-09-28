//! A craft's account read forward through the day-long starlight segments it will be settled at,
//! as the authority settles it. The shard fires what these answer and the client counts down to
//! the same instant. See `lightcone/docs/30-the-field.md` §Collapse and §Auto.

use crate::craft::Craft;
use crate::field::Mode;
use crate::fitting::Fitting;
use crate::motion::ShipState;

/// When `craft`'s field reaches `Q_max` by `until_s`, if it does.
pub fn collapse_by(craft: &Craft, until_s: f64) -> Option<f64> {
    first_by(craft, until_s, Fitting::collapse_s)
}

/// When `craft`'s field in Auto next begins a switch by `until_s`, and toward which shade.
pub fn auto_by(craft: &Craft, until_s: f64) -> Option<(f64, Mode)> {
    first_by(craft, until_s, Fitting::auto_s)
}

/// `solve` over each segment in turn, from the settlement, until it answers or `until_s` passes.
fn first_by<T>(craft: &Craft, until_s: f64, solve: impl Fn(&Fitting, &ShipState, f64) -> Option<T>) -> Option<T> {
    let mut ahead: Option<Craft> = None;
    loop {
        let fitting = ahead.as_ref().unwrap_or(craft).fitting()?;
        let since_s = fitting.since_s();
        let segment_end_s = crate::solar::segment_end(since_s);
        if let Some(found) = solve(fitting, &ahead.as_ref().unwrap_or(craft).motion, segment_end_s.min(until_s)) {
            return Some(found);
        }
        if segment_end_s >= until_s {
            return None;
        }
        let next = ahead.get_or_insert_with(|| craft.clone());
        next.settle(segment_end_s);
        if next.fitting().is_none_or(|f| f.since_s() <= since_s) {
            return None;
        }
    }
}
