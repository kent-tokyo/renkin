#![forbid(unsafe_code)]

pub mod chem_env;
pub mod score;
pub mod scorer;
pub mod search;

#[cfg(feature = "python")]
pub mod python;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// Default set of commercially available starting materials (SMILES).
pub const DEFAULT_BUILDING_BLOCKS: &[&str] = &[
    // ── Simple inorganic / small organics ────────────────────────────────────
    "N",     // ammonia
    "O",     // water
    "C=C",   // ethylene
    "ClCCl", // dichloromethane
    // ── Simple aliphatic reagents ─────────────────────────────────────────────
    "CC(=O)O",          // acetic acid
    "CC(=O)Cl",         // acetyl chloride
    "CCO",              // ethanol
    "CO",               // methanol
    "C(=O)O",           // formic acid
    "CCC(=O)O",         // propionic acid
    "CCCC(=O)O",        // butyric acid
    "CC(C)(C)C(=O)O",   // pivalic acid
    "CCN",              // ethylamine
    "CCCN",             // n-propylamine
    "CCCCN",            // n-butylamine
    "CC(C)N",           // isopropylamine
    "CNC",              // dimethylamine
    "CCN(CC)CC",        // triethylamine
    "OCC",              // ethanol (alt notation, same as CCO)
    "OC(C)C",           // isopropanol
    "OCCO",             // ethylene glycol
    "OCCCO",            // 1,3-propanediol
    "NCCN",             // ethylenediamine
    "BrCC",             // bromoethane
    "BrCCC",            // 1-bromopropane
    "BrCCCBr",          // 1,4-dibromobutane
    "ClCC",             // chloroethane
    "ClCCC",            // 1-chloropropane
    "OC(=O)CCl",        // chloroacetic acid
    "OC(=O)CBr",        // bromoacetic acid
    "OCC(=O)O",         // glycolic acid
    "CC(O)C(=O)O",      // lactic acid
    "OC(=O)CC(=O)O",    // malonic acid
    "OC(=O)CCC(=O)O",   // succinic acid
    "OC(=O)CCCC(=O)O",  // glutaric acid
    "OC(=O)CCCCC(=O)O", // adipic acid
    // ── Amino acids ───────────────────────────────────────────────────────────
    "NCC(=O)O",                    // glycine
    "NC(C)C(=O)O",                 // alanine
    "NC(CC(=O)O)C(=O)O",           // aspartic acid
    "NC(CCC(=O)O)C(=O)O",          // glutamic acid
    "NC(Cc1ccccc1)C(=O)O",         // phenylalanine
    "NC(Cc1ccc(O)cc1)C(=O)O",      // tyrosine
    "NC(CS)C(=O)O",                // cysteine
    "NC(CCCCN)C(=O)O",             // lysine
    "NC(Cc1c[nH]c2ccccc12)C(=O)O", // tryptophan
    // ── Protecting-group reagents ─────────────────────────────────────────────
    "CC(C)(C)OC(=O)Cl",              // Boc-Cl
    "CC(C)(C)OC(=O)OC(=O)OC(C)(C)C", // Boc2O (simplified)
    "ClC(=O)OCc1ccccc1",             // Cbz-Cl
    // ── Phenol / benzoic acid family ─────────────────────────────────────────
    "Oc1ccccc1",          // phenol
    "Oc1ccccc1C(=O)O",    // salicylic acid
    "c1ccccc1C(=O)O",     // benzoic acid
    "Fc1ccc(C(=O)O)cc1",  // 4-fluorobenzoic acid
    "Clc1ccc(C(=O)O)cc1", // 4-chlorobenzoic acid
    "OC(=O)c1ccccn1",     // nicotinic acid (pyridine-3-COOH)
    "OC(=O)c1ccncc1",     // isonicotinic acid (pyridine-4-COOH)
    "OC(=O)c1cnccn1",     // pyrazine-2-carboxylic acid
    "OC(=O)c1ncccn1",     // pyrimidine-4-carboxylic acid
    "CCOC(=O)c1ccccc1",   // ethyl benzoate
    // ── Anilines / aryl amines ────────────────────────────────────────────────
    "c1ccc(N)cc1",             // aniline
    "Nc1ccc(O)cc1",            // 4-aminophenol
    "Nc1ccc(C(=O)O)cc1",       // 4-aminobenzoic acid
    "c1ccc(N)cc1C(=O)O",       // anthranilic acid (2-amino)
    "c1cc(N)ccc1C(=O)O",       // 3-aminobenzoic acid
    "Nc1ccc(F)cc1",            // 4-fluoroaniline
    "Nc1ccc(Cl)cc1",           // 4-chloroaniline
    "Nc1ccc([N+](=O)[O-])cc1", // 4-nitroaniline
    "Nc1ccccc1[N+](=O)[O-]",   // 2-nitroaniline
    "NCc1ccccc1",              // benzylamine
    "NCc1ccccn1",              // 2-aminomethylpyridine
    "Nc1cncnc1",               // 2-aminopyrimidine
    "Nc1cnccn1",               // 2-aminopyrazine
    "Nc1ccccn1",               // 2-aminopyridine
    "Nc1ccncc1",               // 4-aminopyridine
    "Nc1cccnc1",               // 3-aminopyridine
    "c1ccc(Nc2ccccn2)cc1",     // N-phenyl-2-aminopyridine (BB itself)
    // ── Benzaldehydes / aryl ketones ─────────────────────────────────────────
    "O=Cc1ccccc1",               // benzaldehyde
    "O=Cc1ccc(F)cc1",            // 4-fluorobenzaldehyde
    "O=Cc1ccc(Cl)cc1",           // 4-chlorobenzaldehyde
    "O=Cc1ccc(Br)cc1",           // 4-bromobenzaldehyde
    "O=Cc1ccc(OC)cc1",           // 4-methoxybenzaldehyde
    "O=Cc1ccc([N+](=O)[O-])cc1", // 4-nitrobenzaldehyde
    "O=Cc1ccccn1",               // 2-pyridinecarboxaldehyde
    "O=Cc1ccncc1",               // 4-pyridinecarboxaldehyde
    "O=Cc1cnccn1",               // 2-pyrazinecarboxaldehyde
    "O=Cc1ccc(Br)nc1",           // 5-bromopyridine-2-carbaldehyde
    "CC(=O)c1ccccc1",            // acetophenone
    "CC(=O)c1ccccn1",            // 2-acetylpyridine
    "CC(=O)c1ccc(Br)cc1",        // 4-bromoacetophenone
    "CC(=O)c1ccc(F)cc1",         // 4-fluoroacetophenone
    "CC(=O)c1ccc(N)cc1",         // 4-aminoacetophenone
    // ── Simple arenes / hetarenes (Suzuki acceptor fragments) ────────────────
    "c1ccccc1",   // benzene
    "c1ccncc1",   // pyridine (4-isomer)
    "c1ccccn1",   // pyridine (2-isomer)
    "c1ccco1",    // furan
    "c1ccsc1",    // thiophene
    "c1ccnc1",    // pyrimidine fragment
    "c1cnccn1",   // pyrazine
    "c1ncncc1",   // pyrimidine (5-pos context)
    "c1ncc[nH]1", // imidazole
    // ── Aryl halides — phenyl ─────────────────────────────────────────────────
    "Brc1ccccc1",                // bromobenzene
    "Clc1ccccc1",                // chlorobenzene
    "Ic1ccccc1",                 // iodobenzene
    "Brc1ccc(F)cc1",             // 1-bromo-4-fluorobenzene (already Fc1...)
    "Fc1ccc(Br)cc1",             // 1-bromo-4-fluorobenzene
    "Clc1ccc(F)cc1",             // 1-chloro-4-fluorobenzene
    "Brc1ccc(Cl)cc1",            // 1-bromo-4-chlorobenzene
    "Brc1ccc(OC)cc1",            // 4-bromoanisole
    "Brc1ccc([N+](=O)[O-])cc1",  // 4-bromonitrobenzene
    "Brc1ccc(C(=O)O)cc1",        // 4-bromobenzoic acid
    "Brc1ccc(N)cc1",             // 4-bromoaniline
    "Brc1ccc(C(=O)c2ccccc2)cc1", // 4-bromobenzophenone
    "FC(F)(F)c1ccc(Br)cc1",      // 4-bromobenzotrifluoride
    // ── Aryl halides — pyridine family ───────────────────────────────────────
    "Brc1ccncc1",       // 4-bromopyridine
    "Brc1ccccn1",       // 2-bromopyridine
    "Brc1cnccc1",       // 3-bromopyridine
    "Clc1ccncc1",       // 4-chloropyridine
    "Clc1ccccn1",       // 2-chloropyridine
    "Clc1cnccc1",       // 3-chloropyridine
    "Fc1ccncc1",        // 4-fluoropyridine
    "Fc1ccccn1",        // 2-fluoropyridine
    "Brc1cnccn1",       // 2-bromopyrazine
    "Clc1cnccn1",       // 2-chloropyrazine
    "Brc1ncccn1",       // 2-bromopyrimidine
    "Clc1ncccn1",       // 2-chloropyrimidine
    "Brc1ncncc1",       // 5-bromopyrimidine
    "Clc1ncncc1",       // 5-chloropyrimidine
    "Brc1ccc2ncccc2n1", // 2-bromoquinoxaline (simplified)
    "Brc1ccnc2ccccc12", // 4-bromoquinoline
    // ── Aryl halides — thiophene/furan/indole family ──────────────────────────
    "Brc1cccs1",          // 2-bromothiophene
    "Brc1ccco1",          // 2-bromofuran
    "Brc1ccc[nH]1",       // 3-bromopyrrole (N-H form)
    "Brc1cc2ccccc2[nH]1", // 3-bromoindole
    // ── Aryl boronic acids / esters ──────────────────────────────────────────
    "OB(O)c1ccccc1",         // phenylboronic acid
    "OB(O)c1ccncc1",         // pyridine-4-boronic acid
    "OB(O)c1ccccn1",         // pyridine-2-boronic acid
    "OB(O)c1cnccc1",         // pyridine-3-boronic acid
    "OB(O)c1ccco1",          // furan-2-boronic acid
    "OB(O)c1cccs1",          // thiophen-2-boronic acid
    "OB(O)c1ccc(F)cc1",      // 4-fluorophenylboronic acid
    "OB(O)c1ccc(Cl)cc1",     // 4-chlorophenylboronic acid
    "OB(O)c1ccc(OC)cc1",     // 4-methoxyphenylboronic acid
    "OB(O)c1cnccn1",         // pyrazine-2-boronic acid
    "OB(O)c1ncccn1",         // pyrimidine-2-boronic acid
    "OB(O)c1ccc(C(=O)O)cc1", // 4-carboxyphenylboronic acid
    // ── Heteroaromatic amines ─────────────────────────────────────────────────
    "Nc1ccccn1",      // 2-aminopyridine
    "Nc1ccncc1",      // 4-aminopyridine
    "Nc1cccnc1",      // 3-aminopyridine
    "Nc1ncccn1",      // 2-aminopyrimidine
    "Nc1cnccn1",      // 2-aminopyrazine
    "Nc1cc2ccccc2n1", // 2-aminobenzimidazole (simplified)
    // ── Benzyl halides / haloalkyls ───────────────────────────────────────────
    "BrCc1ccccc1", // benzyl bromide
    "ClCc1ccccc1", // benzyl chloride
    "ICc1ccccc1",  // benzyl iodide
    "BrCc1ccccn1", // 2-picolyl bromide
    "BrCc1ccncc1", // 4-picolyl bromide
    // ── Acyl chlorides ────────────────────────────────────────────────────────
    "ClC(=O)c1ccccc1",  // benzoyl chloride
    "ClC(=O)c1ccccn1",  // nicotinoyl chloride
    "ClC(=O)CC(=O)Cl",  // malonyl dichloride
    "ClC(=O)CCC(=O)Cl", // succinyl dichloride
    "ClC(=O)CC(C)=O",   // acetoacetyl chloride
];

/// 46-entry set kept for WASM size constraint testing; see DEFAULT_BUILDING_BLOCKS for the full set.
#[allow(dead_code)]
const _BB_COUNT_CHECK: usize = 0; // DEFAULT_BUILDING_BLOCKS has ~300 entries

#[cfg(test)]
mod trace_test;
