#![forbid(unsafe_code)]

use renkin::DEFAULT_BUILDING_BLOCKS;
use std::path::Path;
use std::process::Command;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn check(label: &str, status: &str) {
    println!("{label:<24} {status}");
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
        let count = std::fs::read_to_string(templates_path)
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0);
        check(
            "Templates",
            &format!("OK ({count} rules)  {templates_path}"),
        );
    } else {
        check("Templates", &format!("not found  {templates_path}"));
    }

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
