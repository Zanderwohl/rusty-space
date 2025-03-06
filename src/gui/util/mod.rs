use std::fs;
use std::path::PathBuf;
use serde::de::DeserializeOwned;

pub fn ensure_folder(path: &PathBuf) -> Result<(), std::io::Error> {
    fs::create_dir_all(path)
}

pub fn ensure_folders(paths: &[&PathBuf]) -> Result<(), std::io::Error> {
    for path in paths {
        let _ = ensure_folder(path);
    }
    Ok(())
}

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
