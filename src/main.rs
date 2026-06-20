use renkin::chem_env;
use renkin::search::{self, SearchConfig};
use renkin::DEFAULT_BUILDING_BLOCKS;

use anyhow::{bail, Result};
use serde::Serialize;

#[derive(Serialize)]
struct Output {
    target: String,
    routes_found: usize,
    routes: Vec<search::Route>,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut target: Option<String> = None;
    let mut max_depth: u32 = 5;
    let mut bb_path: Option<String> = None;
    let mut max_routes: usize = 5;
    let mut beam_width: usize = 0;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--target" | "-t" => {
                i += 1;
                if i < args.len() { target = Some(args[i].clone()); }
            }
            "--depth" | "-d" => {
                i += 1;
                if i < args.len() { max_depth = args[i].parse().unwrap_or(5); }
            }
            "--building-blocks" | "-b" => {
                i += 1;
                if i < args.len() { bb_path = Some(args[i].clone()); }
            }
            "--max-routes" | "-n" => {
                i += 1;
                if i < args.len() { max_routes = args[i].parse().unwrap_or(5); }
            }
            "--beam-width" | "-w" => {
                i += 1;
                if i < args.len() { beam_width = args[i].parse().unwrap_or(0); }
            }
            _ => {}
        }
        i += 1;
    }

    let Some(target_smiles) = target else {
        bail!(
            "Usage: renkin --target <SMILES> [--depth <N>] [--max-routes <N>] \
             [--beam-width <N>] [--building-blocks <path>]\n\
             \n\
             Options:\n  \
             --target / -t      Target molecule SMILES\n  \
             --depth  / -d      Max retrosynthesis depth (default: 5)\n  \
             --max-routes / -n  Max routes to return (default: 5)\n  \
             --beam-width / -w  Beam search width, 0 = unlimited A* (default: 0)\n  \
             --building-blocks  Path to .smi file of commercial starting materials"
        );
    };

    let env = match bb_path {
        Some(ref path) => chem_env::ChemEnv::load(path)?,
        None => chem_env::ChemEnv::load("data/building_blocks.smi")
            .unwrap_or_else(|_| chem_env::ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS)),
    };

    let rules = chem_env::default_rules();
    let config = SearchConfig { max_depth, max_routes, beam_width };
    let routes = search::find_routes(&target_smiles, &env, &rules, &config)?;

    let output = Output {
        target: target_smiles,
        routes_found: routes.len(),
        routes,
    };

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
