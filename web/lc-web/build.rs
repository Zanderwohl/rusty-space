//! Bakes the site build id in at compile time.
//!
//! A docker build context routinely lacks `.git`, so the id arrives as an environment
//! variable and the git call is only the local-development convenience.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SITE_BUILD");

    // Without these, this script runs once and the baked build id never changes again -- which
    // silently disables the whole point of `/v/<build>/`: assets are served `immutable`, so a
    // stylesheet edit would never reach anyone who had already loaded the old one. It went
    // unnoticed for three phases because a local dev server sends `no-store`, and only showed
    // up as a page mysteriously using CSS from several commits ago.
    //
    // A missing `.git` is normal (that is the Docker case, where SITE_BUILD is passed in), so
    // a watch that cannot be set is not an error.
    if let Some(git) = find_git_dir() {
        println!("cargo:rerun-if-changed={}", git.join("HEAD").display());
        // HEAD names a ref; the ref is what moves on an ordinary commit.
        if let Ok(head) = std::fs::read_to_string(git.join("HEAD"))
            && let Some(refname) = head.strip_prefix("ref: ").map(str::trim)
        {
            println!("cargo:rerun-if-changed={}", git.join(refname).display());
        }
    }

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

/// The real git directory, following the one-line `gitdir:` pointer a worktree uses.
fn find_git_dir() -> Option<std::path::PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join(".git");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if candidate.is_file() {
            let contents = std::fs::read_to_string(&candidate).ok()?;
            let path = contents.strip_prefix("gitdir:")?.trim();
            return Some(std::path::PathBuf::from(path));
        }
        if !dir.pop() {
            return None;
        }
    }
}
