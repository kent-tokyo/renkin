//! Process-level contracts for the small standalone CLI utilities.

use std::io::Write;
use std::process::{Command, Stdio};

fn canonicalize(input: &str, args: &[&str]) -> Vec<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_renkin-canonicalize"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start canonicalizer");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .expect("write SMILES input");
    let output = child.wait_with_output().expect("read canonicalizer output");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn canonicalizer_keeps_one_output_line_per_input() {
    let output = canonicalize("CCO\n\nC1(CC\nOCC\n", &[]);
    assert_eq!(output.len(), 4);
    assert_ne!(output[0], "ERR");
    assert_eq!(output[1], "ERR");
    assert_eq!(output[2], "ERR");
    assert_eq!(output[0], output[3]);
}

#[test]
fn canonicalizer_clears_atom_maps_only_when_requested() {
    let mapped = canonicalize("[CH3:1]O\n", &[]);
    assert_eq!(mapped.len(), 1);
    assert!(mapped[0].contains(":1"), "{mapped:?}");

    let cleared = canonicalize("[CH3:1]O\n", &["--clear-atom-maps"]);
    let unmapped = canonicalize("CO\n", &[]);
    assert_eq!(cleared, unmapped);
    assert!(!cleared[0].contains(":1"));
}

#[test]
fn doctor_reports_missing_optional_assets_without_failing() {
    let output = Command::new(env!("CARGO_BIN_EXE_renkin-doctor"))
        .current_dir(std::env::temp_dir())
        .output()
        .expect("start doctor");
    assert!(output.status.success());
    let report = String::from_utf8(output.stdout).unwrap();
    assert!(report.contains(concat!("RENKIN ", env!("CARGO_PKG_VERSION"))));
    assert!(report.contains("Templates"));
    assert!(report.contains("Building blocks"));
    assert!(report.contains("WASM package"));
}
