use clap::Parser;
use csv::{ReaderBuilder, WriterBuilder};
use std::path::{Path, PathBuf};
use std::process;

#[derive(Parser)]
#[command(about = "Sort a CSV file by a column")]
struct Args {
    /// Path to the input CSV file
    file: PathBuf,

    /// Column header to sort by (defaults to "dist", falling back to "distance")
    #[arg(long, short)]
    key: Option<String>,

    /// Output file path (defaults to {filename}_{key}_sort.csv)
    #[arg(long, short)]
    out: Option<PathBuf>,

    /// Keep only the top N rows (plus header)
    #[arg(short)]
    n: Option<usize>,
}

fn main() {
    let args = Args::parse();

    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .from_path(&args.file)
        .unwrap_or_else(|e| {
            eprintln!("Failed to open {}: {e}", args.file.display());
            process::exit(1);
        });

    let headers = reader.headers().unwrap_or_else(|e| {
        eprintln!("Failed to read headers: {e}");
        process::exit(1);
    }).clone();

    let sort_key = resolve_key(&headers, args.key.as_deref());
    let col_idx = headers.iter().position(|h| h == sort_key).unwrap_or_else(|| {
        eprintln!("Column \"{sort_key}\" not found in headers");
        process::exit(1);
    });

    let mut records: Vec<csv::StringRecord> = reader
        .records()
        .filter_map(|r| r.ok())
        .collect();

    records.sort_by(|a, b| {
        let va = a.get(col_idx).unwrap_or("");
        let vb = b.get(col_idx).unwrap_or("");
        match (va.parse::<f64>(), vb.parse::<f64>()) {
            (Ok(fa), Ok(fb)) => fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal),
            _ => va.cmp(vb),
        }
    });

    if let Some(n) = args.n {
        records.truncate(n);
    }

    let out_path = args.out.unwrap_or_else(|| default_output_path(&args.file, &sort_key));

    let mut writer = WriterBuilder::new()
        .from_path(&out_path)
        .unwrap_or_else(|e| {
            eprintln!("Failed to create {}: {e}", out_path.display());
            process::exit(1);
        });

    writer.write_record(&headers).unwrap();
    for record in &records {
        writer.write_record(record).unwrap();
    }
    writer.flush().unwrap();

    println!("Wrote {} rows to {}", records.len(), out_path.display());
}

fn resolve_key(headers: &csv::StringRecord, requested: Option<&str>) -> String {
    if let Some(k) = requested {
        return k.to_string();
    }
    for candidate in &["dist", "distance"] {
        if headers.iter().any(|h| h == *candidate) {
            return candidate.to_string();
        }
    }
    eprintln!("No default sort column found (tried \"dist\", \"distance\"). Use --key to specify one.");
    process::exit(1);
}

fn default_output_path(input: &Path, key: &str) -> PathBuf {
    let stem = input.file_stem().unwrap_or_default().to_string_lossy();
    let parent = input.parent().unwrap_or(Path::new("."));
    parent.join(format!("{stem}_{key}_sort.csv"))
}
