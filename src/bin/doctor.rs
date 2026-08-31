#![forbid(unsafe_code)]

use renkin::DEFAULT_BUILDING_BLOCKS;
use renkin::io_limits::{read_bounded_bytes_file, read_bounded_text_file};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn check(label: &str, status: &str) {
    println!("{label:<24} {status}");
}

/// Verify `local_path` (if present) against `manifest_path`'s
/// `assets.<asset_filename>.sha256` entry. `local_path` not existing is
/// informational, not an error -- e.g. `templates_2000.smi` is excluded
/// from the crates.io package (`Cargo.toml`'s `[package].exclude`), so a
/// `cargo install`/`pip install` user genuinely never has it locally unless
/// they ran the matching `scripts/fetch_*.py`. Never panics regardless of
/// file/manifest state -- a missing or malformed manifest is reported the
/// same soft way, not a crash.
fn check_asset_hash(label: &str, local_path: &str, manifest_path: &str, asset_filename: &str) {
    if !Path::new(local_path).exists() {
        check(label, &format!("not found  {local_path}"));
        return;
    }
    let manifest_text = match read_bounded_text_file(manifest_path, "manifest") {
        Ok(t) => t,
        Err(e) => {
            check(
                label,
                &format!("manifest unreadable  {manifest_path} ({e})"),
            );
            return;
        }
    };
    let manifest: serde_json::Value = match serde_json::from_str(&manifest_text) {
        Ok(v) => v,
        Err(e) => {
            check(
                label,
                &format!("manifest invalid JSON  {manifest_path} ({e})"),
            );
            return;
        }
    };
    let Some(expected) = manifest
        .get("assets")
        .and_then(|a| a.get(asset_filename))
        .and_then(|a| a.get("sha256"))
        .and_then(|s| s.as_str())
    else {
        check(
            label,
            &format!("manifest missing assets.{asset_filename:?}.sha256  {manifest_path}"),
        );
        return;
    };
    let expected = expected.strip_prefix("sha256:").unwrap_or(expected);
    let bytes = match read_bounded_bytes_file(local_path, "asset") {
        Ok(b) => b,
        Err(e) => {
            check(label, &format!("unreadable  {local_path} ({e})"));
            return;
        }
    };
    let actual = renkin::sha256_hex(Sha256::digest(&bytes));
    if actual == expected {
        check(label, "OK (sha256 verified)");
    } else {
        check(
            label,
            &format!("MISMATCH  expected {expected}, got {actual}  {local_path}"),
        );
    }
}

fn probe_binary(name: &str) -> &'static str {
    match Command::new(name).arg("--version").output() {
        Ok(o) if o.status.success() => "OK",
        _ => "not found",
    }
}

fn probe_python() -> String {
    match Command::new("python3")
        .args(["-c", "import renkin; print(renkin.__version__)"])
        .output()
    {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            format!("OK (v{v})")
        }
        _ => "not found".to_string(),
    }
}

fn main() {
    println!("RENKIN {VERSION}\n");

    // Templates
    let templates_path = "data/templates_extracted_5000.smi";
    if Path::new(templates_path).exists() {
        let count = read_bounded_text_file(templates_path, "templates")
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
        check(
            "Templates",
            &format!("OK ({count} rules)  {templates_path}"),
        );
    } else {
        check("Templates", &format!("not found  {templates_path}"));
    }

    // Asset hashes (Issue #101/v0.24 -- opt-in reranker/coverage-mode assets)
    check_asset_hash(
        "Reranker model",
        "data/phase3e_reranker_training/model.txt",
        "data/phase3e_reranker_training/release_asset_manifest.json",
        "model.txt",
    );
    check_asset_hash(
        "Reranker freq table",
        "data/phase3e_reranker_training/frequency_table.json",
        "data/phase3e_reranker_training/release_asset_manifest.json",
        "frequency_table.json",
    );
    check_asset_hash(
        "Coverage templates",
        "data/phase_a5_template_scaling/templates/templates_2000.smi",
        "data/phase_a5_template_scaling/templates/coverage_templates_release_asset_manifest.json",
        "templates_2000.smi",
    );

    // Building blocks
    let bb_count = DEFAULT_BUILDING_BLOCKS.len();
    check("Building blocks", &format!("OK ({bb_count})"));

    // Companion binaries
    check("renkin-forward", probe_binary("renkin-forward"));
    check("renkin-mcp", probe_binary("renkin-mcp"));

    // WASM package
    let wasm_status = if Path::new("pkg/renkin_bg.wasm").exists() {
        "OK"
    } else {
        "not built  (run: wasm-pack build --target web --no-default-features)"
    };
    check("WASM package", wasm_status);

    // Python bindings
    check("Python bindings", &probe_python());
}
