//! Shared fixtures for the integration tests.
//!
//! Each test binary uses a subset of this, so unused items are expected.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Where the bundled system lives in the repository. Read-only as far as tests are
/// concerned — see [`ScratchFile::bundled_solar_system`].
pub fn bundled_solar_system_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/systems/solar_system.em")
}

/// A file under the temp directory that deletes itself when dropped, even if the test
/// panics first.
pub struct ScratchFile(PathBuf);

/// Distinguishes scratch files made in the same process on the same thread.
static NEXT: AtomicU64 = AtomicU64::new(0);

impl ScratchFile {
    /// An empty scratch path. `tag` only aids debugging; uniqueness comes from the
    /// process, the thread and a counter, so parallel tests never share one.
    pub fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "em-test-{tag}-{}-{:?}-{}.em",
            std::process::id(),
            std::thread::current().id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = std::fs::remove_file(&path);
        Self(path)
    }

    /// A private copy of the bundled solar system, for tests that want to load it.
    ///
    /// Loading an `.em` goes through `save_sqlite::open_em_file`, which runs any pending
    /// migrations against the file it opened. The bundled asset sits below the current
    /// schema version, so *reading* it rewrites it: the working tree comes back dirty,
    /// and — because the tests in a binary run in parallel — two threads race the same
    /// `ALTER TABLE`, so whichever loses fails with "duplicate column name". Copying
    /// first gives every test its own file to migrate.
    pub fn bundled_solar_system(tag: &str) -> Self {
        let scratch = Self::new(tag);
        std::fs::copy(bundled_solar_system_path(), &scratch.0)
            .expect("the bundled save should be readable");
        scratch
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn to_path_buf(&self) -> PathBuf {
        self.0.clone()
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
