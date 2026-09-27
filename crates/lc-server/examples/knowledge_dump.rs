//! What one craft believes about the bodies of a system, from its stored knowledge rows.
//!
//! Reads a CSV of `lc_knowledge` rows, one per line as `subject_hex,format,file_hex,saved_t`,
//! which `psql -At -F,` writes from
//! `SELECT encode(subject,'hex'), format, encode(file,'hex'), saved_t FROM lc_knowledge WHERE ship_id = <id>`.
//! Prints each body's orbit, its sigmas, and the pieces its error bars are drawn from, at the
//! newest `saved_t` or at `--at <coordinate seconds>`. Bodies of Sol are named.
//!
//! `cargo run -p lc-server --example knowledge_dump -- <rows.csv> <ship_id> [--at <s>]`

use std::collections::HashMap;

use lc_world::knowledge::{BodyId, Knowledge, Method, Orientation, Placed, Subject, Witness};

const AU_KM: f64 = 1.495_978_707e8;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let (path, ship) = match (args.get(1), args.get(2).and_then(|s| s.parse::<i64>().ok())) {
        (Some(path), Some(ship)) => (path, ship),
        _ => return Err("usage: knowledge_dump <rows.csv> <ship_id> [--at <coordinate seconds>]".into()),
    };
    let at = args.iter().position(|a| a == "--at").and_then(|i| args.get(i + 1)?.parse::<f64>().ok());

    let text = std::fs::read_to_string(path).map_err(|why| format!("{path}: {why}"))?;
    let mut files = Vec::new();
    let mut newest_t = i64::MIN;
    for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        let fields: Vec<&str> = line.trim().split(',').collect();
        let [subject, format, file, saved_t] = fields[..] else {
            return Err(format!("line {}: expected 4 fields, got {}", n + 1, fields.len()));
        };
        let row = lc_store::knowledge::Filed {
            ship_id: ship,
            subject: hex(subject).map_err(|why| format!("line {}: {why}", n + 1))?,
            format: format.parse().map_err(|_| format!("line {}: format {format}", n + 1))?,
            file: hex(file).map_err(|why| format!("line {}: {why}", n + 1))?,
            saved_t: saved_t.parse().map_err(|_| format!("line {}: saved_t {saved_t}", n + 1))?,
        };
        newest_t = newest_t.max(row.saved_t);
        match lc_server::archive::read_file(&row) {
            Ok(file) => files.push(file),
            Err(why) => eprintln!("line {}: skipped, {why}", n + 1),
        }
    }
    let now_s = at.unwrap_or(newest_t as f64 * 1.0e-6);
    let knowledge = Knowledge::restore(Witness(ship as u64), files, Vec::new());
    println!("ship {ship}: {} subjects, at coordinate {now_s:.0} s", knowledge.len());

    let stars: Vec<_> = knowledge
        .files()
        .filter_map(|(s, _)| match s {
            Subject::Body { star, .. } => Some(star),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    for star in stars {
        let names = sol_names(star);
        let name = |body: BodyId| names.get(&body).cloned().unwrap_or_else(|| format!("{:016x}", body.get()));
        println!("\nstar {:#x}", star.get());
        let mut beliefs = knowledge.bodies_of(star, now_s);
        beliefs.sort_by_key(|b| name(b.body));
        for belief in beliefs {
            let subject = Subject::Body { star, body: belief.body };
            let file = knowledge.file(subject);
            let orbit = file.and_then(|f| {
                f.orbits().iter().max_by(|a, b| {
                    (a.witness == knowledge.owner)
                        .cmp(&(b.witness == knowledge.owner))
                        .then(a.method.standing().cmp(&b.method.standing()))
                        .then(a.stated_s.total_cmp(&b.stated_s))
                })
            });
            let looks = file.map_or(0, |f| f.sightings().len());
            println!("{:>16}  looks {looks:>2}", name(belief.body));
            let Some(o) = orbit else {
                println!("{:>16}  no orbit", "");
                continue;
            };
            let whose = if o.witness == knowledge.owner { "own" } else if o.method == Method::Claim { "chart/claim" } else { "reported" };
            let about = o.about.map_or("star".to_string(), &name);
            let rel = |(v, s): (f64, f64)| if v != 0.0 { s / v.abs() } else { s };
            let pole = match o.orientation {
                Orientation::Known { sigma_rad, .. } => format!("{sigma_rad:.1e} rad"),
                Orientation::EdgeOnTo { .. } => "edge-on only".into(),
                Orientation::Unknown => "unknown".into(),
            };
            println!(
                "{:>16}  {whose} {:?} about {about}: a {:.6} AU (±{:.1e} of it), P {:.3} d (±{:.1e}), e {}, pole ±{pole}, epoch ±{}, pivot {}",
                "",
                o.method,
                o.semi_major_au.0,
                rel(o.semi_major_au),
                o.period_s.0 / 86_400.0,
                rel(o.period_s),
                o.eccentricity.map_or("none".into(), |(e, s)| format!("{e:.4}±{s:.1e}")),
                o.epoch_s.map_or("none".into(), |(_, s)| format!("{s:.1} s")),
                o.pivot_s.map_or("none".into(), |p| format!("{:.2} d ago", (now_s - p) / 86_400.0)),
            );
            match belief.position_now {
                Placed::Known { error, .. } => println!(
                    "{:>16}  error: outward {:.3e} AU, out of plane {:.3e} AU, sideways {:.3e} AU, along {:.3e} rad = {:.3e} AU{}; total {:.3e} AU = {:.0} km",
                    "",
                    error.outward_au(),
                    error.normal_au(),
                    error.sigma_au(error.sideways()),
                    error.along_rad,
                    error.along_au(),
                    if error.anywhere_on_orbit() { " (anywhere on its orbit)" } else { "" },
                    error.total_au(),
                    error.total_au() * AU_KM,
                ),
                Placed::Shell { radius_au, sigma_au } => {
                    println!("{:>16}  shell: radius {radius_au:.4} AU ± {sigma_au:.3e} AU", "")
                }
                Placed::Unknown => println!("{:>16}  not placed", ""),
            }
        }
    }
    Ok(())
}

fn hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.strip_prefix("\\x").unwrap_or(s);
    if s.len() % 2 != 0 {
        return Err("odd-length hex".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2).unwrap_or(""), 16).map_err(|why| why.to_string()))
        .collect()
}

/// Every body of Sol by its id, if `star` is Sol; empty otherwise.
fn sol_names(star: lc_world::sky::StarId) -> HashMap<BodyId, String> {
    let catalog = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/catalogs/hygdata_v42_dist_sort.csv");
    let Ok(provider) = lc_world::sky::hyg::HygProvider::load(catalog) else { return HashMap::new() };
    let Some(sun) = lc_world::sky::StarProvider::stars(&provider)
        .iter()
        .find(|s| s.provenance.name.as_deref() == Some(lc_world::system::SOL))
        .cloned()
    else {
        return HashMap::new();
    };
    if sun.id != star {
        return HashMap::new();
    }
    let Some(system) = lc_world::system::LocalSystem::for_star(&sun) else { return HashMap::new() };
    let sim = system.sim();
    sim.indices()
        .map(|i| {
            let name = sim.info(i).name.clone().unwrap_or_else(|| sim.name(i).to_string());
            (BodyId::of(star, &name), name)
        })
        .collect()
}
