//! Migrate save files from TOML format to SQLite (.em) format.
//!
//! This tool reads all save files from ./data/saves and ./data/templates
//! and writes them as .em files in the same directories.

use std::fs;
use std::path::PathBuf;

use exotic_matters::body::universe::save::{UniverseFile, SaveFormat};
use exotic_matters::body::universe::save_sqlite;

fn main() {
    println!("=== Exotic Matters Save Migration Tool ===\n");

    let directories = vec![
        PathBuf::from("data/saves"),
        PathBuf::from("data/templates"),
    ];

    let mut total_migrated = 0;
    let mut total_errors = 0;

    for dir in directories {
        if !dir.exists() {
            println!("Directory {:?} does not exist, skipping.", dir);
            continue;
        }

        println!("Processing directory: {:?}", dir);

        match fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    
                    // Skip directories and .em files
                    if path.is_dir() {
                        continue;
                    }
                    
                    // Only process TOML files
                    let format = SaveFormat::from_path(&path);
                    if format != Some(SaveFormat::Toml) {
                        continue;
                    }

                    let file_name = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");

                    print!("  Migrating: {} ... ", file_name);

                    match migrate_file(&path) {
                        Ok(new_path) => {
                            println!("OK -> {:?}", new_path.file_name().unwrap_or_default());
                            total_migrated += 1;
                        }
                        Err(e) => {
                            println!("ERROR: {}", e);
                            total_errors += 1;
                        }
                    }
                }
            }
            Err(e) => {
                println!("  Error reading directory: {}", e);
            }
        }

        println!();
    }

    println!("=== Migration Complete ===");
    println!("  Successfully migrated: {}", total_migrated);
    println!("  Errors: {}", total_errors);
}

fn migrate_file(source_path: &PathBuf) -> Result<PathBuf, String> {
    // Load the TOML file
    let universe_file = UniverseFile::load_from_path(source_path)
        .ok_or_else(|| format!("Failed to load file"))?;

    // Create the new .em path
    let new_path = source_path.with_extension("em");

    // Save as .em
    save_sqlite::save_to_em(&new_path, &universe_file.contents)
        .map_err(|e| format!("{:?}", e))?;

    Ok(new_path)
}

