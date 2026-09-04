//! Writing the bundled presets to disk.
//!
//! The preset data itself lives in `em_sim::presets` — it is plain data with no engine
//! dependency, so a headless tool can build a system without going through a file.

use std::path::PathBuf;
use crate::body::universe::save::UniverseFile;
use crate::gui::util::ensure_folders;

pub use em_sim::presets::{earth_moon, solar_system, EARTH_MOON_PATH, SOLAR_SYSTEM_PATH};

fn write_preset(contents: em_sim::universe::UniverseFileContents, path: &str) {
    let dir = PathBuf::from("data/templates");
    ensure_folders(&[&dir]).expect("Folders couldn't be made");
    let file = UniverseFile { file: Some(PathBuf::from(path)), contents };
    file.save().expect("Failed to save system");
}

pub fn write_temp_system_file() {
    write_preset(solar_system(), SOLAR_SYSTEM_PATH);
}

pub fn write_earth_moon_file() {
    write_preset(earth_moon(), EARTH_MOON_PATH);
}
