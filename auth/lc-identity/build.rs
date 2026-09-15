//! Compiles the stylesheet into the binary.
//!
//! At build time, not at boot as the site does it. The broker ships one small sheet that
//! nothing reloads, so compiling it here buys three things the site cannot have: a SCSS error
//! is a failed build rather than a failed deploy, the image needs no `static/` tree, and the
//! cache-busting segment can be a digest of the CSS itself rather than a git revision — which
//! is what makes the `immutable` header below unconditionally true.

use std::path::Path;

fn main() {
    let styles = Path::new("static/styles");
    // Cargo watches a directory recursively, so a new partial is picked up without this being
    // edited. Without it the sheet is compiled once and every later edit is invisible.
    println!("cargo:rerun-if-changed={}", styles.display());

    let entry = styles.join("application.scss");
    let css = match grass::from_path(&entry, &grass::Options::default()) {
        Ok(css) => css,
        Err(why) => panic!("compiling {}: {why}", entry.display()),
    };

    let out = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("identity.css");
    std::fs::write(&out, &css).expect("writing the compiled stylesheet");
    println!("cargo:rustc-env=STYLE_DIGEST={}", digest(css.as_bytes()));
}

/// FNV-1a over the compiled CSS, as hex.
///
/// A cache key, not a security boundary: it only has to change when the bytes do, and a
/// 64-bit non-cryptographic hash does that without a dependency.
fn digest(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}
