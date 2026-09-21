//! Where the device grant is kept between launches.
//!
//! The OS keychain, which is what `lightcone/docs/16-identity.md` asks for — with a file
//! beside the configuration as a fallback, because a headless Linux box has no secret service
//! and "sign in again every launch" is a worse answer than a file only its owner can read.

use std::path::PathBuf;

/// What the keychain entry is filed under.
const SERVICE: &str = "lightcone";
const ACCOUNT: &str = "device-grant";

#[derive(Debug)]
pub enum VaultError {
    Unavailable(String),
    Io(String),
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultError::Unavailable(why) => write!(f, "no keychain: {why}"),
            VaultError::Io(why) => write!(f, "{why}"),
        }
    }
}

/// Somewhere to keep one secret.
pub enum Vault {
    Keychain,
    /// Owner-readable only. See [`Vault::file`].
    File(PathBuf),
    /// For tests, and for a run that should leave nothing behind.
    Memory(std::sync::Mutex<Option<String>>),
}

impl Vault {
    /// The keychain if there is one, a file if there is not.
    ///
    /// Decided by *trying* it rather than by inspecting the platform: a desktop Linux with a
    /// running secret service and a container without one are the same target triple, and only
    /// one of them has anywhere to put this.
    pub fn best(config_dir: PathBuf) -> Self {
        match keyring::Entry::new(SERVICE, ACCOUNT).and_then(|entry| match entry.get_password() {
            // Not finding one is the ordinary first-run case, and proves the keychain works.
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other.map(|_| ()),
        }) {
            Ok(()) => Vault::Keychain,
            Err(why) => {
                bevy::log::warn!("no keychain ({why}); keeping the sign-in in a file instead");
                Vault::file(config_dir)
            }
        }
    }

    pub fn file(config_dir: PathBuf) -> Self {
        Vault::File(config_dir.join("device-grant"))
    }

    pub fn memory() -> Self {
        Vault::Memory(std::sync::Mutex::new(None))
    }

    pub fn read(&self) -> Result<Option<String>, VaultError> {
        match self {
            Vault::Keychain => {
                match keyring::Entry::new(SERVICE, ACCOUNT).and_then(|entry| entry.get_password()) {
                    Ok(secret) => Ok(Some(secret)),
                    Err(keyring::Error::NoEntry) => Ok(None),
                    Err(why) => Err(VaultError::Unavailable(why.to_string())),
                }
            }
            Vault::File(path) => match std::fs::read_to_string(path) {
                Ok(secret) => Ok(Some(secret.trim().to_owned()))
                    .map(|s: Option<String>| s.filter(|secret| !secret.is_empty())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(VaultError::Io(e.to_string())),
            },
            Vault::Memory(held) => Ok(held.lock().unwrap().clone()),
        }
    }

    pub fn write(&self, secret: &str) -> Result<(), VaultError> {
        match self {
            Vault::Keychain => keyring::Entry::new(SERVICE, ACCOUNT)
                .and_then(|entry| entry.set_password(secret))
                .map_err(|why| VaultError::Unavailable(why.to_string())),
            Vault::File(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| VaultError::Io(e.to_string()))?;
                }
                std::fs::write(path, secret).map_err(|e| VaultError::Io(e.to_string()))?;
                restrict(path)
            }
            Vault::Memory(held) => {
                *held.lock().unwrap() = Some(secret.to_owned());
                Ok(())
            }
        }
    }

    /// Forget it. What signing out does, and what a rejected grant triggers.
    pub fn clear(&self) -> Result<(), VaultError> {
        match self {
            Vault::Keychain => match keyring::Entry::new(SERVICE, ACCOUNT)
                .and_then(|entry| entry.delete_credential())
            {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(why) => Err(VaultError::Unavailable(why.to_string())),
            },
            Vault::File(path) => match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(VaultError::Io(e.to_string())),
            },
            Vault::Memory(held) => {
                *held.lock().unwrap() = None;
                Ok(())
            }
        }
    }
}

/// Owner read and write, nothing else.
///
/// A ninety-day credential in a world-readable file is a ninety-day credential for everyone
/// with an account on the machine. Best effort elsewhere: Windows has no mode bits and its
/// per-user profile directory is the protection.
#[cfg(unix)]
fn restrict(path: &std::path::Path) -> Result<(), VaultError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| VaultError::Io(e.to_string()))
}

#[cfg(not(unix))]
fn restrict(_path: &std::path::Path) -> Result<(), VaultError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lc-vault-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_secret_survives_being_written_and_read() {
        for vault in [Vault::memory(), Vault::file(scratch("round-trip"))] {
            assert_eq!(vault.read().unwrap(), None, "something was there already");
            vault.write("a-device-grant").unwrap();
            assert_eq!(vault.read().unwrap().as_deref(), Some("a-device-grant"));
            vault.clear().unwrap();
            assert_eq!(vault.read().unwrap(), None);
            // Clearing something already gone is not an error: it is what signing out does
            // twice, and what a rejected grant does to one that was never stored.
            vault.clear().unwrap();
        }
    }

    #[test]
    fn a_later_write_replaces_the_earlier_one() {
        let vault = Vault::file(scratch("replace"));
        vault.write("first").unwrap();
        vault.write("second").unwrap();
        assert_eq!(vault.read().unwrap().as_deref(), Some("second"));
        vault.clear().unwrap();
    }

    /// A ninety-day credential readable by everyone with an account on the machine is a
    /// ninety-day credential for everyone with an account on the machine.
    #[cfg(unix)]
    #[test]
    fn the_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("permissions");
        let vault = Vault::file(dir.clone());
        vault.write("a-device-grant").unwrap();

        let Vault::File(path) = &vault else {
            unreachable!()
        };
        let mode = std::fs::metadata(path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// An empty file is nothing rather than an empty grant, which would be sent to the broker
    /// and refused on every launch.
    #[test]
    fn an_empty_file_holds_nothing() {
        let dir = scratch("empty");
        let vault = Vault::file(dir.clone());
        vault.write("").unwrap();
        assert_eq!(vault.read().unwrap(), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
