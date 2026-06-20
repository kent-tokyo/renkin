pub mod chem_env;
pub mod score;
pub mod search;

#[cfg(feature = "python")]
pub mod python;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// Default set of commercially available starting materials (SMILES).
pub const DEFAULT_BUILDING_BLOCKS: &[&str] = &[
    "CC(=O)O",              // acetic acid
    "CC(=O)Cl",             // acetyl chloride
    "Oc1ccccc1",            // phenol
    "Oc1ccccc1C(=O)O",      // salicylic acid
    "c1ccccc1",             // benzene
    "c1ccc(N)cc1",          // aniline
    "N",                    // ammonia
    "O",                    // water
    "CCO",                  // ethanol
    "CO",                   // methanol
    "C(=O)O",               // formic acid
    "CC",                   // ethane
    "C",                    // methane
    "ClCCl",                // dichloromethane
    "c1ccccc1C(=O)O",       // benzoic acid
    "NCC(=O)O",             // glycine
    "OCC(=O)O",             // glycolic acid
    "CC(O)C(=O)O",          // lactic acid
    "OC(=O)CCC(=O)O",       // succinic acid
    "OC(=O)CC(=O)O",        // malonic acid
    "c1ccc(N)cc1C(=O)O",    // anthranilic acid
    "c1cc(N)ccc1C(=O)O",    // 3-aminobenzoic acid
];
