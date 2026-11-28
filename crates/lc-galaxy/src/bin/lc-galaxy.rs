//! ```text
//! lc-galaxy check   <timeline.ron>
//! lc-galaxy stats   <timeline.ron> [--box <x0,y0,z0:x1,y1,z1>]
//! lc-galaxy publish <timeline.ron> --out <dir> [--name <name>] [--threads <n>]
//!                   [--onto <manifest.galaxy> [--rebake <x,y,z>]...] [--region <x0,y0,z0:x1,y1,z1>]
//! lc-galaxy diff    <timeline.ron> --against <manifest.galaxy> [--threads <n>]
//! lc-galaxy picture <timeline.ron> --out <png> [--side +x|-x|+y|-y|+z|-z] [--size <w>x<h>]
//!                   [--resolution <voxels>] [--decades <n>] [--saturation <k>] [--star-exposure <stops>|off]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use lc_galaxy::sample::default_threads;
use lc_galaxy::{Aabb, Registry, Row, codec, publish, stats};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    // The first argument after the command that is neither a flag nor a flag's value.
    let path = args.iter().enumerate().skip(1).find(|(i, a)| !a.starts_with("--") && !matches!(args[i - 1].as_str(), "--out" | "--name" | "--against" | "--onto" | "--region" | "--rebake" | "--box" | "--threads" | "--side" | "--size" | "--resolution" | "--decades" | "--saturation" | "--star-exposure")).map(|(_, a)| a);
    let (Some(command), Some(path)) = (args.first(), path) else {
        eprintln!("usage: lc-galaxy check|stats|publish|diff|picture <timeline.ron> [--box <x0,y0,z0:x1,y1,z1>] [--out <path>] [--against <manifest>] [--name <name>] [--threads <n>] [--side <side>] [--size <w>x<h>] [--resolution <n>] [--decades <n>]");
        return ExitCode::FAILURE;
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let timeline = match codec::read(&text, &Registry::standard()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match command.as_str() {
        "check" => {
            let compiled = timeline.compile(timeline.len(), None);
            let unread = timeline.rows().iter().filter(|r| matches!(r, Row::Unread(_))).count();
            println!("{} rows, {unread} unread, {} problems", timeline.len(), compiled.problems.len());
            for r in publish::refusals(&timeline, &compiled) {
                println!("  {r}");
            }
            println!("graph {:016x}", publish::graph_version(&timeline));
            if unread == 0 && compiled.problems.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
        }
        "stats" => {
            let region = match after("--box").map(|b| parse_box(&b)) {
                Some(Ok(region)) => region,
                Some(Err(e)) => {
                    eprintln!("--box: {e}");
                    return ExitCode::FAILURE;
                }
                None => timeline.compile(timeline.len(), None).bounds(),
            };
            match stats::report(&timeline, region, default_threads()) {
                Ok(text) => {
                    print!("{text}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        "publish" => {
            let Some(out) = after("--out") else {
                eprintln!("publish needs --out <dir>");
                return ExitCode::FAILURE;
            };
            let name = after("--name").unwrap_or_else(|| {
                PathBuf::from(path).file_stem().map_or("galaxy".into(), |s| s.to_string_lossy().into_owned())
            });
            // The output must not depend on this; tests/cli.rs publishes with 1 and with many.
            let threads = match after("--threads").map(|t| t.parse::<usize>()) {
                Some(Ok(n)) if n > 0 => n,
                Some(_) => {
                    eprintln!("--threads needs a whole number above zero");
                    return ExitCode::FAILURE;
                }
                None => default_threads(),
            };
            let region = match after("--region").map(|b| parse_box(&b)) {
                Some(Ok(r)) => Some(r),
                Some(Err(e)) => {
                    eprintln!("--region: {e}");
                    return ExitCode::FAILURE;
                }
                None => None,
            };
            let mut rebake = Vec::new();
            for value in args.windows(2).filter(|w| w[0] == "--rebake").map(|w| &w[1]) {
                match value.split(',').map(|v| v.trim().parse::<i32>()).collect::<Result<Vec<_>, _>>() {
                    Ok(v) if v.len() == 3 => rebake.push([v[0], v[1], v[2]]),
                    _ => {
                        eprintln!("--rebake {value}: not a chunk x,y,z");
                        return ExitCode::FAILURE;
                    }
                }
            }
            let onto = after("--onto").map(PathBuf::from);
            if onto.is_none() && !rebake.is_empty() {
                eprintln!("--rebake needs --onto <manifest.galaxy>");
                return ExitCode::FAILURE;
            }
            let options = publish::Options { onto, region, rebake };
            match publish::publish_with(&timeline, &name, &PathBuf::from(&out), threads, &options) {
                Ok(m) => {
                    let stars: u32 = m.chunks.iter().map(|c| c.stars).sum();
                    println!("{name}: {stars} stars in {} chunk file(s), graph {:016x}, manifest {}, to {out}", m.chunks.len(), publish::graph_version(&timeline), m.hash());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        "diff" => {
            let Some(against) = after("--against") else {
                eprintln!("diff needs --against <manifest.galaxy>");
                return ExitCode::FAILURE;
            };
            let threads = after("--threads").and_then(|t| t.parse().ok()).filter(|&n| n > 0).unwrap_or_else(default_threads);
            match diff(&timeline, &PathBuf::from(&against), threads) {
                Ok((text, differs)) => {
                    print!("{text}");
                    if differs { ExitCode::FAILURE } else { ExitCode::SUCCESS }
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        "picture" => match picture(&timeline, &after) {
            Ok(line) => {
                println!("{line}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        other => {
            eprintln!("unknown command {other}");
            ExitCode::FAILURE
        }
    }
}

fn picture(timeline: &lc_galaxy::Timeline, after: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    use lc_galaxy::picture::{Bright, Side, View, Volume, march, sprinkle};
    let out = after("--out").ok_or("picture needs --out <png>")?;
    let side = match after("--side") {
        Some(s) => Side::named(&s).ok_or(format!("--side {s}: not one of +x, -x, +y, -y, +z, -z"))?,
        None => Side::named("+z").unwrap(),
    };
    let (width, height) = match after("--size") {
        Some(s) => {
            let (w, h) = s.split_once('x').ok_or(format!("--size {s}: needs <width>x<height>"))?;
            (w.parse::<u32>().map_err(|e| format!("--size {s}: {e}"))?, h.parse::<u32>().map_err(|e| format!("--size {s}: {e}"))?)
        }
        None => (800, 800),
    };
    let number = |flag: &str, default: f64| after(flag).map_or(Ok(default), |v| v.parse::<f64>().map_err(|e| format!("{flag} {v}: {e}")));
    let resolution = number("--resolution", 192.0)? as usize;
    let decades = number("--decades", 3.0)?;
    let saturation = number("--saturation", 1.2)?;
    let compiled = timeline.compile(timeline.len(), None);
    if !compiled.problems.is_empty() {
        return Err(format!("{} features have problems; `lc-galaxy check` lists them", compiled.problems.len()));
    }
    let volume = Volume::of(&compiled, resolution, default_threads());
    let view = View::fitting(side, volume.bounds(), width, height);
    let (mut display, peak) = march(&volume, &view, width, height, default_threads()).display(decades, saturation);
    if after("--star-exposure").as_deref() != Some("off") {
        let exposure = number("--star-exposure", 0.0)?;
        let baked = lc_galaxy::bake(&compiled, timeline.seed, compiled.bounds(), default_threads()).map_err(|e| e.to_string())?;
        let stars: Vec<Bright> = baked.stars.iter().filter(|s| s.tier == lc_galaxy::sample::Tier::A).map(|s| Bright { position_ly: s.position_ly, luminosity_solar: s.attributes.luminosity_solar, teff_k: s.attributes.teff_k }).collect();
        sprinkle(&mut display, width, height, &view, &stars, peak, exposure, 1.0);
    }
    let rgba = lc_galaxy::picture::encode(&display);
    let file = std::fs::File::create(&out).map_err(|e| format!("{out}: {e}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(&rgba)).map_err(|e| format!("{out}: {e}"))?;
    Ok(format!("{out}: {width}x{height} from {}, {} ly per pixel, volume {}x{}x{} at {:.2} ly", side.label(), view.ly_per_px, volume.dims.x, volume.dims.y, volume.dims.z, volume.voxel_ly))
}

/// `x0,y0,z0:x1,y1,z1` in ly, the corners in either order.
fn parse_box(text: &str) -> Result<Aabb, String> {
    let corner = |s: &str| -> Result<glam::DVec3, String> {
        let v: Vec<f64> = s.split(',').map(|c| c.trim().parse::<f64>().map_err(|e| format!("{c:?}: {e}"))).collect::<Result<_, _>>()?;
        match v.as_slice() {
            [x, y, z] => Ok(glam::DVec3::new(*x, *y, *z)),
            _ => Err(format!("{s:?} is not three numbers")),
        }
    };
    let (a, b) = text.split_once(':').ok_or("needs two corners separated by ':'")?;
    let (a, b) = (corner(a)?, corner(b)?);
    Ok(Aabb { min: a.min(b), max: a.max(b) })
}

/// One line per chunk and tier, and whether any published chunk differs beyond tolerance.
fn diff(timeline: &lc_galaxy::Timeline, against: &std::path::Path, threads: usize) -> Result<(String, bool), String> {
    use lc_galaxy::diff::{Status, Tolerance};
    use std::fmt::Write;
    let (manifest, chunks) = lc_world::sky::galaxy::Manifest::read_from_disk(against)?;
    let compiled = timeline.compile(timeline.len(), None);
    let refused = publish::refusals(timeline, &compiled);
    if !refused.is_empty() {
        return Err(format!("not comparable:\n  {}", refused.join("\n  ")));
    }
    let bake = lc_galaxy::bake(&compiled, timeline.seed, compiled.bounds(), threads).map_err(|e| e.to_string())?;
    let published: Vec<_> = manifest.chunks.into_iter().zip(chunks).collect();
    let rows = lc_galaxy::diff::diff(&compiled, &bake, publish::graph_version(timeline), &published, Tolerance::default())?;
    let mut out = String::new();
    let _ = writeln!(out, "{:<16} {:<4} {:<11} {:>9} {:>6} {:>6} {:>7} {:>8} {:>7}", "chunk", "tier", "status", "published", "now", "kept", "changed", "Δ[Fe/H]", "Δshare");
    for r in &rows {
        let [x, y, z] = r.chunk;
        let _ = writeln!(
            out,
            "{:<16} {:<4} {:<11} {:>9} {:>6} {:>6} {:>7} {:>+8.3} {:>7.3}",
            format!("{x},{y},{z}"),
            format!("{:?}", r.tier),
            r.status.label(),
            r.published,
            r.now,
            r.kept,
            r.changed,
            r.feh_shift,
            r.share_shift
        );
    }
    let count = |s: Status| rows.iter().filter(|r| r.status == s).count();
    let _ = writeln!(
        out,
        "{} same, {} close, {} differ, {} unpublished",
        count(Status::Same),
        count(Status::Close),
        count(Status::Differs),
        count(Status::Unpublished)
    );
    Ok((out, count(Status::Differs) > 0))
}
