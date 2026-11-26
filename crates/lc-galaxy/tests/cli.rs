//! The `lc-galaxy` binary, run as a separate process.

use std::process::Command;

fn lc_galaxy(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lc-galaxy")).args(args).output().expect("lc-galaxy runs")
}

fn phase_one() -> String {
    format!("{}/galaxies/phase-1.ron", env!("CARGO_MANIFEST_DIR"))
}

/// The report has the counts, the shells and the systems table, and its baked total is the
/// population rows' sum.
#[test]
fn stats_reports_a_whole_galaxy() {
    let out = lc_galaxy(&["stats", &phase_one()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    for wanted in ["population", "total", "nearest neighbor:", "shell radius:", "planets / star", "ocean-first"] {
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
