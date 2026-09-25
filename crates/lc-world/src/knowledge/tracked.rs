//! A value whose every write is counted, so a cache built from it can tell when to rebuild.

use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};

/// One counter for every instance in the process, so a `Knowledge` replaced wholesale — by a
/// snapshot, or a new session — can never come back with a revision a cache already holds.
static REVISIONS: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    REVISIONS.fetch_add(1, Ordering::Relaxed)
}

/// Read through `Deref`; written only through [`Tracked::edit`], which moves the revision
/// whether or not the caller then changes anything.
#[derive(Clone, Debug)]
pub(crate) struct Tracked<T> {
    inner: T,
    revision: u64,
}

impl<T> Tracked<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self { inner, revision: next() }
    }

    pub(crate) fn edit(&mut self) -> &mut T {
        self.revision = next();
        &mut self.inner
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
}

impl<T> Deref for Tracked<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T: PartialEq> PartialEq for Tracked<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}
