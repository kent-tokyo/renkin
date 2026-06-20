#[cfg(test)]
mod tests {
    use crate::chem_env::{ChemEnv, apply_retro, default_rules, mol_from_smiles, to_canonical};

    const SIMPLE_UNSOLVED: &[(&str, &str)] = &[
        (
            "[CH3]N([CH3])[CH2][CH]1[CH2][CH2][CH2][CH2]C1=O",
            "NMe2-ketone",
        ),
        (
            "O=[CH]c1[cH][cH][cH]c(-c2[cH][cH][cH]o2)n1",
            "pyridine-furan-biaryl",
        ),
        (
            "[CH3][NH]c1[cH]c([CH3])c(OC([CH3])=O)c([CH3])c1[CH3]",
            "aryl-amine-ester",
        ),
        (
            "[CH3]C(=O)[CH2]c1[cH][cH]c2c([cH]1)O[CH2]O2",
            "ketone-arene",
        ),
        (
            "[CH3]c1[cH]nc([CH3])c([CH3])c1[Cl]",
            "chloro-methylpyridine",
        ),
    ];

    #[test]
    fn trace_pipeline() {
        let env = ChemEnv::load("data/building_blocks.smi").unwrap();
        let rules = default_rules();

        for (smiles, desc) in SIMPLE_UNSOLVED {
            println!("\n=== TARGET: {} | {} ===", desc, smiles);

            let mol = match mol_from_smiles(smiles) {
                Ok(m) => m,
                Err(e) => {
                    println!("  PARSE ERROR: {:?}", e);
                    continue;
                }
            };
            let canon = to_canonical(&mol);
            println!("  canonical: {}", canon);
            println!("  is_BB: {}", env.is_building_block(&mol));

            let mut any_rule_fired = false;
            for rule in &rules {
                let precursors = apply_retro(&mol, rule);
                if precursors.is_empty() {
                    continue;
                }
                any_rule_fired = true;
                println!("  RULE [{}]:", rule.name);
                for (i, set) in precursors.iter().enumerate().take(3) {
                    let parts: Vec<String> = set
                        .iter()
                        .map(|p| {
                            let is_bb = env.is_building_block(&p.mol);
                            format!("{} (BB={})", p.smiles, is_bb)
                        })
                        .collect();
                    println!("    [{}] {}", i, parts.join(" | "));
                }
            }
            if !any_rule_fired {
                println!("  NO RULES FIRED");
            }
        }
    }

    #[test]
    fn check_explicit_h_matching() {
        let env = ChemEnv::in_memory(&["CC(=O)O", "c1ccccc1", "Cc1ccccc1", "Clc1ccccc1"]);

        let test_cases = &[
            ("[CH3]C(=O)O", "explicit-H acetic acid vs CC(=O)O"),
            ("c1ccccc1", "benzene exact"),
            ("[cH]1[cH][cH][cH][cH][cH]1", "explicit-H benzene"),
            ("C(=O)(O)c1ccccc1", "benzoic acid not in BB"),
        ];

        for (smi, desc) in test_cases {
            let mol = mol_from_smiles(smi).unwrap();
            let is_bb = env.is_building_block(&mol);
            println!("  {} => is_BB={} ({})", smi, is_bb, desc);
        }
    }
}
