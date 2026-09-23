//! Packs a star catalog into a sky chunk.
//!
//! Offline, and native only. The browser build consumes what this writes and never sees a
//! CSV; this is the only program that reads one.
//!
//!     cargo run -p lc-world --bin skypack -- assets/catalogs/hygdata_v42.csv out.lcsky
//!
//! `--limit N` keeps the N nearest stars. The client sorts by distance and truncates anyway,
//! so packing more than it will use is bytes nobody downloads for a reason. Pack a little
//! above the client's limit, not exactly at it, so raising the limit does not silently start
//! rendering a shorter sky.

use std::path::PathBuf;

use lc_world::sky::hyg::HygProvider;
use lc_world::sky::{chunk, record};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut limit: Option<usize> = None;
    let mut positional: Vec<&str> = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--limit" => limit = rest.next().and_then(|n| n.parse().ok()),
            other if other.starts_with("--") => {
                eprintln!("unknown option {other}");
                std::process::exit(2);
            }
            other => positional.push(other),
        }
    }

    let Some(input) = positional.first().map(PathBuf::from) else {
        eprintln!("usage: skypack <catalog.csv> [out.lcsky] [--limit N]");
        std::process::exit(2);
    };
    let output = positional
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| input.with_extension("lcsky"));

    let mut records = HygProvider::read_records(&input)?;
    let total = records.len();
    if let Some(limit) = limit {
        // The same ordering the client applies, so the chunk is the head of the list it would
        // have kept rather than an arbitrary subset of it.
        records.sort_by(|a, b| a.position_ly.length().total_cmp(&b.position_ly.length()));
        records.truncate(limit);
    }
    let (bytes, dropped) = chunk::pack(lc_world::sky::hyg::SOURCE, &records)?;
    std::fs::write(&output, &bytes)?;

    // Decode what was just written and assemble it, so the program cannot report success on a
    // chunk that does not read back.
    let (source, back) = chunk::decode(&bytes)?;
    let (stars, skipped) = record::assemble_all(&source, &back);

    let csv_bytes = std::fs::metadata(&input)?.len();
    println!(
        "{} -> {}\n  {} rows read, {} considered, {} packed, {} rejected, {} lost to rounding\n  {:.1} MB CSV -> {:.2} MB chunk, {} bytes per star",
        input.display(),
        output.display(),
        total,
        records.len(),
        stars.len(),
        dropped,
        skipped,
        csv_bytes as f64 / 1.0e6,
        bytes.len() as f64 / 1.0e6,
        bytes.len() / stars.len().max(1),
    );
    Ok(())
}
