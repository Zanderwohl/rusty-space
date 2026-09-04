pub mod format;
/// Float bit-punning. Only used to derive the SQLite `time_key` column — a stable
/// integer key for an exact float time. Not a domain concern.
pub mod bitfutz;
pub mod ease;

/// Moved into `em-foundations`; re-exported so `crate::util::…` paths keep resolving.
pub use em_foundations::{common, mappings, patched_conics};
/// Moved into `em-sim`; re-exported so `crate::util::…` paths keep resolving.
pub use em_sim::time_map;

use std::fs;
use std::path::PathBuf;
use serde::de::DeserializeOwned;

/// Create a directory (and all parent directories) if it doesn't exist.
pub fn ensure_folder(path: &PathBuf) -> Result<(), std::io::Error> {
    fs::create_dir_all(path)
}

/// Create multiple directories.
pub fn ensure_folders(paths: &[&PathBuf]) -> Result<(), std::io::Error> {
    for path in paths {
        let _ = ensure_folder(path);
    }
    Ok(())
}

/// Ensure a TOML file exists at the given path.
/// If it doesn't exist, creates it with default values.
/// Returns the parsed content.
pub fn ensure_toml<T: Default + DeserializeOwned + serde::ser::Serialize>(path: &PathBuf) -> Result<T, std::io::Error> {
    match fs::exists(path) {
        Ok(exists) => {
            let toml_values = if !exists {
                let default = T::default();
                fs::write(path, toml::to_string_pretty(&default).unwrap())?;
                Ok(default)
            } else {
                let file_string = fs::read_to_string(&path)?;
                toml::from_str::<T>(file_string.as_str()).map_err(|err| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData,
                                        format!("failed to parse toml: {}", err))
                })
            };
            toml_values
        }
        Err(err) => Err(err)
    }
}
