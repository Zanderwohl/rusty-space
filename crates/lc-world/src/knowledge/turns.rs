//! Which turn a look is on.
//!
//! How many whole orbits passed between two looks is not in the anomalies. A survey's cadence
//! usually makes it obvious -- mean anomaly advances uniformly, so the forward walk reads it
//! off -- but a cadence near the period does not, and the wrong turn count is a period that is
//! wrong by a ratio rather than by an error bar.
//!
//! So this hands back every rate the anomalies could be saying, and [`super::arc::through`]
//! scores them against the bearings. It cannot be settled here: putting each look on the turn
//! nearest a candidate rate makes *every* rate regress well, which is the aliasing it was meant
//! to resolve.

use super::arc::sound;

/// Turn counts tried when the forward walk is not obviously right. See [`unwrappings`].
const TURNS_TRIED: u32 = 12;

/// An anomaly step this large between consecutive looks makes the shortest way round a guess
/// rather than a reading, and the turn count has to be searched.
const AMBIGUOUS_STEP_RAD: f64 = std::f64::consts::FRAC_PI_2;

/// Every rate and epoch the anomalies could be saying, best guess first.
///
/// The case this exists for: twelve looks seven tenths of an orbit apart: the forward walk
/// reads each step as three tenths *backwards* and reports a period 2.33 times the truth, at a
/// residual thirty-five thousand times the bearing noise, and nothing downstream refused it.
///
/// Searched only when a step is over [`AMBIGUOUS_STEP_RAD`], so a well-sampled arc costs one
/// candidate. A survey's cadence is nowhere near this for a period worth fitting.
pub(crate) fn unwrappings(mean: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    let walk = walked(mean);
    if let Some(found) = regress(&walk) {
        out.push((found.0, found.1));
    }
    let ambiguous = walk
        .windows(2)
        .any(|pair| matches!(pair, [a, b] if (b.0 - a.0).abs() > AMBIGUOUS_STEP_RAD));
    let (Some(first), Some(last), true) = (mean.first(), walk.last(), ambiguous) else {
        return out;
    };
    let span_s = last.1 - first.1;
    let reach = last.0 - first.0;
    for turns in 1..=TURNS_TRIED {
        for direction in [1.0, -1.0] {
            let total = reach + direction * std::f64::consts::TAU * f64::from(turns);
            let rate = span_s / total;
            if !sound(rate.abs()) {
                continue;
            }
            if let Some(found) = regress(&placed(mean, *first, rate)) {
                out.push((found.0, found.1));
            }
        }
    }
    out
}

/// The anomalies unwrapped by walking forward, which is right when consecutive looks are under
/// half an orbit apart.
fn walked(mean: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut unwrapped: Vec<(f64, f64)> = Vec::with_capacity(mean.len());
    let mut turns = 0.0;
    let Some(mut last) = mean.first().map(|(m, _)| *m) else { return unwrapped };
    for (m, t) in mean {
        if *m + turns < last - std::f64::consts::PI {
            turns += std::f64::consts::TAU;
        }
        last = m + turns;
        unwrapped.push((last, *t));
    }
    unwrapped
}

/// The anomalies unwrapped against a rate: each put on the turn nearest where that rate says
/// it should be. `rate` is seconds per radian.
fn placed(mean: &[(f64, f64)], first: (f64, f64), rate: f64) -> Vec<(f64, f64)> {
    mean.iter()
        .map(|(m, t)| {
            let expected = first.0 + (t - first.1) / rate;
            let turns = ((expected - m) / std::f64::consts::TAU).round();
            (m + turns * std::f64::consts::TAU, *t)
        })
        .collect()
}

/// Least squares of `t` on `M`. Returns seconds per radian, the epoch, and the root mean square
/// of what is left, which is what tells one unwrapping from another.
fn regress(unwrapped: &[(f64, f64)]) -> Option<(f64, f64, f64)> {
    let count = unwrapped.len() as f64;
    if count < 2.0 {
        return None;
    }
    let mean_m = unwrapped.iter().map(|(m, _)| m).sum::<f64>() / count;
    let mean_t = unwrapped.iter().map(|(_, t)| t).sum::<f64>() / count;
    let mut var = 0.0;
    let mut cov = 0.0;
    for (m, t) in unwrapped {
        var += (m - mean_m) * (m - mean_m);
        cov += (m - mean_m) * (t - mean_t);
    }
    if !sound(var) {
        return None;
    }
    let per_rad = cov / var;
    if !sound(per_rad.abs()) {
        return None;
    }
    let epoch_s = mean_t - per_rad * mean_m;
    // Scaled by the rate, so what is compared is the anomaly left over in radians rather than
    // a time, which would flatter a short period.
    let left: f64 = unwrapped
        .iter()
        .map(|(m, t)| {
            let miss = (t - epoch_s) / per_rad - m;
            miss * miss
        })
        .sum();
    Some((per_rad, epoch_s, (left / count).sqrt()))
}


#[cfg(test)]
mod tests {
    use super::*;
    use em_foundations::kepler;

    /// An arc too coarse to unwrap one way is unwrapped every way and scored.
    ///
    /// What this does not fix, and cannot: an arc sampled at a *fixed* fraction of the period
    /// is stroboscopic, and for a circular orbit the aliases put the body in the same places at
    /// the same times. Irregular or denser sampling is what breaks that, and a real survey has
    /// it.
    #[test]
    fn a_coarsely_sampled_arc_offers_every_unwrapping() {
        let period = 3.156e7;
        let at = |fraction: f64| {
            (0..12)
                .map(|k| {
                    let t = k as f64 * period * fraction;
                    (kepler::anomaly::wrap_pi(std::f64::consts::TAU * t / period), t)
                })
                .collect::<Vec<(f64, f64)>>()
        };
        let implied = |rate: f64| rate.abs() * std::f64::consts::TAU / period;

        // A tenth of an orbit apart: nothing to search, and the one answer is the right one.
        let dense = unwrappings(&at(0.1));
        assert_eq!(dense.len(), 1, "a tight arc is not ambiguous and must not cost a search");
        assert!((implied(dense[0].0) - 1.0).abs() < 1.0e-9, "{}", implied(dense[0].0));

        // Seven tenths apart: the forward walk cannot see it, and the truth is in the set
        // behind it for the bearings to pick out.
        let coarse = unwrappings(&at(0.7));
        assert!(coarse.len() > 1, "an ambiguous arc has to be searched");
        assert!((implied(coarse[0].0) - 1.0).abs() > 0.1, "the forward walk is the wrong one");
        let closest =
            coarse.iter().map(|(r, _)| (implied(*r) - 1.0).abs()).fold(f64::INFINITY, f64::min);
        assert!(closest < 1.0e-6, "the truth is not among the candidates: off by {closest}");
    }
}
