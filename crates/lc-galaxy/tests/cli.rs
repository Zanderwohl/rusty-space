//! The `lc-galaxy` binary, run as a separate process.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn lc_galaxy(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lc-galaxy")).args(args).output().expect("lc-galaxy runs")
}

fn phase_one() -> String {
    format!("{}/galaxies/phase-1.ron", env!("CARGO_MANIFEST_DIR"))
}

/// Every file under `dir`, by its path relative to `dir`.
fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for entry in std::fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else {
                let name = path.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                out.insert(name, std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// Doc 34's determinism rule: the same graph publishes byte-identical files in separate
/// processes, with one thread and with many. Phase II crosses cell faces and has two populations
/// and system fields, so the thinning, the attributes and the profile tables all take part.
#[test]
fn publishing_is_byte_identical_across_processes_and_thread_counts() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("publish-determinism");
    let _ = std::fs::remove_dir_all(&root);
    let galaxy = format!("{}/galaxies/phase-2.ron", env!("CARGO_MANIFEST_DIR"));
    let publish = |threads: &str| {
        let out = root.join(format!("threads-{threads}"));
        let run = lc_galaxy(&["publish", &galaxy, "--out", out.to_str().unwrap(), "--threads", threads]);
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        files(&out)
    };
    let (one, many) = (publish("1"), publish("7"));
    assert!(one.contains_key("manifest.galaxy") && one.len() > 2, "{:?}", one.keys().collect::<Vec<_>>());
    assert_eq!(one.keys().collect::<Vec<_>>(), many.keys().collect::<Vec<_>>(), "the two publishes wrote different files");
    for (name, bytes) in &one {
        assert!(bytes == &many[name], "{name} differs between 1 thread and 7");
    }
}

/// The report has the counts, the shells and the systems table, and its baked total is the
/// population rows' sum.
#[test]
fn stats_reports_a_whole_galaxy() {
    let out = lc_galaxy(&["stats", &phase_one()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    for wanted in ["population", "total", "tier A, over", "nearest neighbor:", "shell radius:", "planets / star", "ocean-first", "moons / star", "planets with a moon"] {
        assert!(text.contains(wanted), "no {wanted:?} in\n{text}");
    }
    let baked = |line: &str| line.split_whitespace().last().and_then(|n| n.parse::<usize>().ok());
    let rows: Vec<&str> = text.lines().skip_while(|l| !l.starts_with("population")).skip(1).take_while(|l| !l.contains("cells")).collect();
    let (total, populations) = rows.split_last().unwrap();
    assert_eq!(baked(total), populations.iter().map(|l| baked(l).unwrap()).sum::<usize>().into());
}

/// A box that is not two corners of three numbers is refused, not read as some other box.
#[test]
fn stats_refuses_a_malformed_box() {
    let out = lc_galaxy(&["stats", &phase_one(), "--box", "1,2:3,4,5"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--box"));
}

/// `diff` against the graph's own publish finds every chunk the same and exits zero; against a
/// different seed's publish it exits nonzero.
#[test]
fn diff_against_its_own_publish_is_all_same() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("diff-cli");
    let _ = std::fs::remove_dir_all(&root);
    let galaxy = format!("{}/galaxies/phase-2.ron", env!("CARGO_MANIFEST_DIR"));
    let out = root.join("own");
    assert!(lc_galaxy(&["publish", &galaxy, "--out", out.to_str().unwrap()]).status.success());
    let manifest = out.join("manifest.galaxy");
    let run = lc_galaxy(&["diff", &galaxy, "--against", manifest.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "{text}{}", String::from_utf8_lossy(&run.stderr));
    assert!(text.contains(" 0 close, 0 differ, 0 unpublished"), "{text}");

    let reseeded = root.join("reseeded.ron");
    std::fs::write(&reseeded, std::fs::read_to_string(&galaxy).unwrap().replacen("seed: ", "seed: 9", 1)).unwrap();
    let run = lc_galaxy(&["diff", reseeded.to_str().unwrap(), "--against", manifest.to_str().unwrap()]);
    assert!(!run.status.success(), "{}", String::from_utf8_lossy(&run.stdout));
}
