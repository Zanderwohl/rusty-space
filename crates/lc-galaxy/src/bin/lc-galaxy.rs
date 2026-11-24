//! ```text
//! lc-galaxy check   <timeline.ron>
//! lc-galaxy publish <timeline.ron> --out <dir> [--name <name>]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use lc_galaxy::{Registry, Row, codec, publish};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    // The first argument after the command that is neither a flag nor a flag's value.
    let path = args.iter().enumerate().skip(1).find(|(i, a)| !a.starts_with("--") && !matches!(args[i - 1].as_str(), "--out" | "--name")).map(|(_, a)| a);
    let (Some(command), Some(path)) = (args.first(), path) else {
        eprintln!("usage: lc-galaxy check|publish <timeline.ron> [--out <dir>] [--name <name>]");
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
        "publish" => {
            let Some(out) = after("--out") else {
                eprintln!("publish needs --out <dir>");
                return ExitCode::FAILURE;
            };
            let name = after("--name").unwrap_or_else(|| {
                PathBuf::from(path).file_stem().map_or("galaxy".into(), |s| s.to_string_lossy().into_owned())
            });
            match publish::publish(&timeline, &name, &PathBuf::from(&out)) {
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
