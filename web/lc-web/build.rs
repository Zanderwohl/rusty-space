//! Bakes the site build id in at compile time.
//!
//! A docker build context routinely lacks `.git`, so the id arrives as an environment
//! variable and the git call is only the local-development convenience.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SITE_BUILD");

    let build =
        std::env::var("SITE_BUILD").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
            Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")))
        });
    println!("cargo:rustc-env=SITE_BUILD={build}");
}
