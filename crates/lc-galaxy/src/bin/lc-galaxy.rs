//! ```text
//! lc-galaxy check   <timeline.ron>
//! lc-galaxy stats   <timeline.ron> [--box <x0,y0,z0:x1,y1,z1>]
//! lc-galaxy publish <timeline.ron> --out <dir> [--name <name>] [--threads <n>]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use lc_galaxy::sample::default_threads;
use lc_galaxy::{Aabb, Registry, Row, codec, publish, stats};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    // The first argument after the command that is neither a flag nor a flag's value.
    let path = args.iter().enumerate().skip(1).find(|(i, a)| !a.starts_with("--") && !matches!(args[i - 1].as_str(), "--out" | "--name" | "--box" | "--threads")).map(|(_, a)| a);
    let (Some(command), Some(path)) = (args.first(), path) else {
        eprintln!("usage: lc-galaxy check|stats|publish <timeline.ron> [--box <x0,y0,z0:x1,y1,z1>] [--out <dir>] [--name <name>] [--threads <n>]");
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
            match publish::publish(&timeline, &name, &PathBuf::from(&out), threads) {
                Ok(m) => {
                    let stars: u32 = m.chunks.iter().map(|c| c.stars).sum();
                    println!("{name}: {stars} stars in {} chunk(s), graph {}, to {out}", m.chunks.len(), m.graph);
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        other => {
            eprintln!("unknown command {other}");
            ExitCode::FAILURE
        }
    }
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
