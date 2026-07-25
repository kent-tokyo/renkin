//! RENKIN Rust quickstart. Compiled and run as part of CI (see
//! .github/workflows/ci.yml) so this example can never silently drift from
//! the real `find_routes` API.

use renkin::chem_env::{ChemEnv, default_rules};
use renkin::search::{SearchConfig, find_routes};

fn main() -> anyhow::Result<()> {
    let env = ChemEnv::load("data/building_blocks.smi")?;
    let rules = default_rules();
    let config = SearchConfig {
        max_depth: 5,
        max_routes: 3,
        ..Default::default()
    };

    let (routes, _stats) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &config)?;
    println!("Routes found: {}", routes.len());
    for route in &routes {
        println!("Route (depth {}):", route.depth);
        for step in &route.steps {
            println!("  {} -> {}", step.target, step.precursors.join(" + "));
            println!("  via {}", step.rule);
        }
    }
    Ok(())
}
