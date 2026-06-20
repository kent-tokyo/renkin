pub mod chem_env;
pub mod score;
pub mod search;

#[cfg(feature = "python")]
pub mod python;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// Default set of commercially available starting materials (SMILES).
pub const DEFAULT_BUILDING_BLOCKS: &[&str] = &[
    // ── Simple reagents ──────────────────────────────────────────────────────
    "CC(=O)O",  // acetic acid
    "CC(=O)Cl", // acetyl chloride
    "N",        // ammonia
    "O",        // water
    "CCO",      // ethanol
    "CO",       // methanol
    "C(=O)O",   // formic acid
    "ClCCl",    // dichloromethane
    "C=C",      // ethylene (Heck acceptor)
    // ── Phenol / benzoic acid family ─────────────────────────────────────────
    "Oc1ccccc1",        // phenol
    "Oc1ccccc1C(=O)O",  // salicylic acid
    "c1ccccc1C(=O)O",   // benzoic acid
    "CCOC(=O)c1ccccc1", // ethyl benzoate
    // ── Aniline family ───────────────────────────────────────────────────────
    "c1ccc(N)cc1",       // aniline
    "Nc1ccc(O)cc1",      // 4-aminophenol (paracetamol precursor)
    "Nc1ccc(C(=O)O)cc1", // 4-aminobenzoic acid
    "c1ccc(N)cc1C(=O)O", // anthranilic acid (2-aminobenzoic acid)
    "c1cc(N)ccc1C(=O)O", // 3-aminobenzoic acid
    // ── Simple arenes / hetarenes (Suzuki acceptor fragments) ────────────────
    "c1ccccc1", // benzene
    "c1ccncc1", // pyridine (4-phenylpyridine acceptor fragment)
    "c1ccccn1", // pyridine (2-substituted)
    "c1ccco1",  // furan (pyridine-furan biaryl acceptor fragment)
    // ── Aryl halides (Suzuki / Buchwald-Hartwig / Heck donors) ───────────────
    "Brc1ccccc1",         // bromobenzene
    "Clc1ccccc1",         // chlorobenzene
    "Fc1ccc(Br)cc1",      // 1-bromo-4-fluorobenzene
    "Brc1ccncc1",         // 4-bromopyridine
    "Brc1ccccn1",         // 2-bromopyridine
    "Brc1cnccc1",         // 3-bromopyridine
    "O=Cc1ccc(Br)nc1",    // 5-bromopyridine-2-carbaldehyde
    "CC(=O)c1ccc(Br)cc1", // 4-bromoacetophenone
    // ── Aryl boronic acids / esters (Suzuki acceptors) ───────────────────────
    "OB(O)c1ccccc1",    // phenylboronic acid
    "OB(O)c1ccncc1",    // pyridine-4-boronic acid
    "OB(O)c1ccccn1",    // pyridine-2-boronic acid
    "OB(O)c1ccco1",     // furan-2-boronic acid
    "OB(O)c1ccc(F)cc1", // 4-fluorophenylboronic acid
    // ── Heteroaromatic amines (Buchwald-Hartwig) ─────────────────────────────
    "Nc1ccccn1", // 2-aminopyridine
    "Nc1ccncc1", // 4-aminopyridine
    // ── Amino acids / small acids ────────────────────────────────────────────
    "NCC(=O)O",       // glycine
    "OCC(=O)O",       // glycolic acid
    "CC(O)C(=O)O",    // lactic acid
    "OC(=O)CCC(=O)O", // succinic acid
    "OC(=O)CC(=O)O",  // malonic acid
];

#[cfg(test)]
mod trace_test;
