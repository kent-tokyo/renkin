//! Contract for direct precursor generators (for example Chemformer-like
//! models). The contract is intentionally separate from `ReactionPrior`:
//! these models propose precursor sets without selecting a RENKIN template.
//!
//! This module does not execute proposals or change the existing search. It
//! defines the evidence that an eventual adapter must carry across the
//! candidate/validation boundary.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Resource and search context supplied to a direct precursor generator.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct RetroGenerationContext {
    pub depth: u32,
    pub max_proposals: usize,
}

/// Stable identity of the model call that produced a proposal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneratorProvenance {
    pub generator_id: String,
    pub generator_version: String,
    pub artifact_sha256: String,
    pub source_rank: usize,
    pub model_confidence: Option<f64>,
}

/// One atom-map relationship carried by a direct generator. Atom indices are
/// local to the corresponding SMILES molecule; chemistry-aware validation
/// resolves them against parsed molecules later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtomMapEntry {
    pub map_num: u32,
    pub target_atom_index: usize,
    pub precursor_index: usize,
    pub precursor_atom_index: usize,
}

/// One direct precursor proposal. `precursors` are kept as strings at this
/// boundary so the generator remains independent of a particular chemistry
/// parser; RENKIN validation owns parsing, stock, forward replay, and route
/// acceptance after this boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetroProposal {
    pub candidate_id: String,
    pub target: String,
    pub precursors: Vec<String>,
    /// None means the upstream generator did not provide mapping evidence;
    /// it is never treated as proof that the proposal is valid.
    pub atom_mapping: Option<Vec<AtomMapEntry>>,
    pub provenance: GeneratorProvenance,
}

/// Convert one direct proposal into the existing route container so callers
/// can send it through RENKIN's normal audit/stock/forward pipeline. This is
/// only a representation bridge: it does not mark the proposal as accepted.
pub fn proposal_to_route(proposal: &RetroProposal) -> crate::search::Route {
    crate::search::Route {
        steps: vec![crate::search::ReactionStep {
            rule: "direct_generator".to_owned(),
            template_id: format!("generator:{}", proposal.provenance.generator_id),
            target: proposal.target.clone(),
            precursors: proposal.precursors.clone(),
            conditions: None,
            atom_economy: None,
            atom_economy_raw_percent: None,
            atom_economy_status: crate::search::AtomEconomyStatus::NotEvaluable,
            step_confidence: proposal.provenance.model_confidence.unwrap_or(0.0),
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        }],
        depth: 1,
        score: 0.0,
        building_blocks: proposal.precursors.clone(),
        confidence: proposal.provenance.model_confidence.unwrap_or(0.0),
        convergency: 1.0,
        success_probability: 0.0,
        route_cost: 0.0,
    }
}

/// Send a direct proposal through the same route normalizer and audit boundary
/// used by native RENKIN routes. The returned report is the only place callers
/// should consult structural, stock, or forward-validation findings; this
/// helper never upgrades a proposal to a successful route.
pub fn audit_proposal(
    proposal: &RetroProposal,
    configured_stock: Option<&HashSet<String>>,
    rules: Option<&[crate::chem_env::RetroRule]>,
) -> crate::bridge::audit::AuditReport {
    let route = proposal_to_route(proposal);
    let outcome = crate::bridge::route_graph::normalize_renkin_route(&route, &proposal.target);
    crate::bridge::audit::audit(&outcome, configured_stock, rules)
}

/// Checked variant of [`audit_proposal`]. It enforces the generator contract
/// before constructing a route document, so callers integrating an external
/// generator cannot accidentally audit malformed or untraceable output.
pub fn audit_proposal_checked(
    proposal: &RetroProposal,
    context: &RetroGenerationContext,
    configured_stock: Option<&HashSet<String>>,
    rules: Option<&[crate::chem_env::RetroRule]>,
) -> Result<crate::bridge::audit::AuditReport, String> {
    let decision = RetroGeneratorDecision {
        proposals: vec![proposal.clone()],
        abstained: false,
    };
    validate_decision(&proposal.target, context, &decision)?;
    validate_chemistry(&proposal.target, &decision)?;
    Ok(audit_proposal(proposal, configured_stock, rules))
}

/// Generator output. `abstained` is distinct from an empty successful result
/// and must cause the caller to retain its configured fallback proposer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetroGeneratorDecision {
    pub proposals: Vec<RetroProposal>,
    pub abstained: bool,
}

/// Direct precursor-generation extension point. Implementations must not
/// claim route success: every proposal still passes RENKIN's structural,
/// stock, forward-validation, and audit pipeline.
pub trait RetroGenerator: Send + Sync {
    fn propose(&self, target: &str, context: &RetroGenerationContext) -> RetroGeneratorDecision;

    /// Validated adapter entry point. Keeping `propose` as the minimal trait
    /// method preserves simple model implementations, while callers that
    /// cross into RENKIN's search boundary can require both transport and
    /// chemistry-aware parse checks before inspecting any proposal.
    fn propose_checked(
        &self,
        target: &str,
        context: &RetroGenerationContext,
    ) -> Result<RetroGeneratorDecision, String> {
        let decision = self.propose(target, context);
        validate_decision(target, context, &decision)?;
        validate_chemistry(target, &decision)?;
        Ok(decision)
    }
}

/// Reference adapter that exposes RENKIN's existing one-step rule proposer
/// through the direct-generator contract. It provides a baseline for future
/// learned generators while retaining stable candidate and source provenance.
pub struct RuleBasedRetroGenerator<'a> {
    context: &'a crate::candidate::CandidateProposalContext<'a>,
    config: &'a crate::candidate::ProposalConfig,
}

impl<'a> RuleBasedRetroGenerator<'a> {
    pub fn new(
        context: &'a crate::candidate::CandidateProposalContext<'a>,
        config: &'a crate::candidate::ProposalConfig,
    ) -> Self {
        Self { context, config }
    }
}

impl RetroGenerator for RuleBasedRetroGenerator<'_> {
    fn propose(&self, target: &str, _context: &RetroGenerationContext) -> RetroGeneratorDecision {
        let Ok(pool) = self.context.propose_one_step(target, target, self.config) else {
            return RetroGeneratorDecision {
                proposals: Vec::new(),
                abstained: true,
            };
        };
        let proposals = pool
            .candidates
            .into_iter()
            .filter_map(|candidate| {
                let source = candidate.sources.first()?;
                Some(RetroProposal {
                    candidate_id: candidate.candidate_id,
                    target: target.to_owned(),
                    precursors: candidate.precursor_smiles,
                    atom_mapping: None,
                    provenance: GeneratorProvenance {
                        generator_id: "renkin-rule-proposer".to_owned(),
                        generator_version: env!("CARGO_PKG_VERSION").to_owned(),
                        artifact_sha256: "not_applicable".to_owned(),
                        source_rank: source.original_rank,
                        model_confidence: source.upstream_score.map(f64::from),
                    },
                })
            })
            .collect();
        RetroGeneratorDecision {
            proposals,
            abstained: false,
        }
    }
}

/// Validate the transport-level contract before chemistry-specific checks.
/// This catches ambiguous or untraceable model output without deciding
/// whether a proposed reaction is chemically valid.
pub fn validate_decision(
    target: &str,
    context: &RetroGenerationContext,
    decision: &RetroGeneratorDecision,
) -> Result<(), String> {
    if target.is_empty() {
        return Err("target must not be empty".to_owned());
    }
    if decision.abstained && !decision.proposals.is_empty() {
        return Err("abstained decision must not contain proposals".to_owned());
    }
    if decision.proposals.len() > context.max_proposals && context.max_proposals != 0 {
        return Err(format!(
            "proposal count {} exceeds max_proposals {}",
            decision.proposals.len(),
            context.max_proposals
        ));
    }

    let mut ids = HashSet::new();
    for proposal in &decision.proposals {
        if proposal.target != target {
            return Err("proposal target differs from generator target".to_owned());
        }
        if proposal.candidate_id.is_empty() {
            return Err("proposal candidate_id must not be empty".to_owned());
        }
        if !ids.insert(&proposal.candidate_id) {
            return Err(format!(
                "duplicate candidate_id {:?}",
                proposal.candidate_id
            ));
        }
        if proposal.precursors.is_empty() || proposal.precursors.iter().any(String::is_empty) {
            return Err("proposal must contain non-empty precursor strings".to_owned());
        }
        if let Some(mapping) = &proposal.atom_mapping {
            let mut map_numbers = HashSet::new();
            for entry in mapping {
                if entry.map_num == 0 {
                    return Err("atom mapping map_num must be positive".to_owned());
                }
                if !map_numbers.insert(entry.map_num) {
                    return Err(format!("duplicate atom mapping map_num {}", entry.map_num));
                }
                if entry.precursor_index >= proposal.precursors.len() {
                    return Err("atom mapping precursor_index is out of range".to_owned());
                }
            }
        }
        let provenance = &proposal.provenance;
        if provenance.generator_id.is_empty()
            || provenance.generator_version.is_empty()
            || provenance.artifact_sha256.is_empty()
        {
            return Err("generator provenance is incomplete".to_owned());
        }
        if provenance
            .model_confidence
            .is_some_and(|confidence| !confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
        {
            return Err("model_confidence must be finite and in [0, 1]".to_owned());
        }
    }
    Ok(())
}

/// Optional chemistry-aware validation after transport validation. This only
/// verifies that the strings and mapping indices are parseable; it does not
/// decide reaction validity, stock membership, or forward replay.
pub fn validate_chemistry(target: &str, decision: &RetroGeneratorDecision) -> Result<(), String> {
    let target_atom_count = crate::chem_env::mol_from_smiles(target)
        .map_err(|error| format!("target SMILES is not parseable: {error}"))?
        .atoms()
        .count();
    for proposal in &decision.proposals {
        let precursor_atom_counts: Vec<usize> = proposal
            .precursors
            .iter()
            .map(|smiles| {
                crate::chem_env::mol_from_smiles(smiles)
                    .map(|molecule| molecule.atoms().count())
                    .map_err(|error| format!("precursor SMILES is not parseable: {error}"))
            })
            .collect::<Result<_, _>>()?;
        if let Some(mapping) = &proposal.atom_mapping {
            for entry in mapping {
                if entry.target_atom_index >= target_atom_count {
                    return Err("atom mapping target_atom_index is out of range".to_owned());
                }
                if entry.precursor_atom_index >= precursor_atom_counts[entry.precursor_index] {
                    return Err("atom mapping precursor_atom_index is out of range".to_owned());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(id: &str) -> RetroProposal {
        RetroProposal {
            candidate_id: id.to_owned(),
            target: "CCO".to_owned(),
            precursors: vec!["CC".to_owned(), "O".to_owned()],
            atom_mapping: None,
            provenance: GeneratorProvenance {
                generator_id: "fixture".to_owned(),
                generator_version: "1".to_owned(),
                artifact_sha256: "sha256:fixture".to_owned(),
                source_rank: 0,
                model_confidence: Some(0.5),
            },
        }
    }

    struct FixtureGenerator {
        decision: RetroGeneratorDecision,
    }

    impl RetroGenerator for FixtureGenerator {
        fn propose(
            &self,
            _target: &str,
            _context: &RetroGenerationContext,
        ) -> RetroGeneratorDecision {
            self.decision.clone()
        }
    }

    #[test]
    fn accepts_traceable_bounded_decision() {
        let decision = RetroGeneratorDecision {
            proposals: vec![proposal("candidate-1")],
            abstained: false,
        };
        assert!(
            validate_decision(
                "CCO",
                &RetroGenerationContext {
                    depth: 0,
                    max_proposals: 2,
                },
                &decision
            )
            .is_ok()
        );
    }

    #[test]
    fn proposal_route_bridge_preserves_candidate_facts_without_claiming_success() {
        let proposal = proposal("candidate-1");
        let route = proposal_to_route(&proposal);
        assert_eq!(route.steps.len(), 1);
        assert_eq!(route.steps[0].rule, "direct_generator");
        assert_eq!(route.steps[0].precursors, proposal.precursors);
        assert_eq!(route.success_probability, 0.0);
        assert_eq!(
            route.steps[0].atom_economy_status,
            crate::search::AtomEconomyStatus::NotEvaluable
        );
    }

    #[test]
    fn proposal_audit_uses_existing_audit_boundary() {
        let proposal = proposal("candidate-1");
        let stock = HashSet::from(["CC".to_owned(), "O".to_owned()]);
        let report = audit_proposal(&proposal, Some(&stock), None);
        assert_eq!(report.steps.len(), 1);
        assert_eq!(report.steps[0].target, "C(C)O");
        assert!(report.reaction_steps_parseable.is_some());
        assert_eq!(report.status, crate::bridge::audit::AuditStatus::Partial);
    }

    #[test]
    fn checked_proposal_audit_rejects_unparseable_output() {
        let mut invalid = proposal("candidate-1");
        invalid.precursors[0] = "not-smiles".to_owned();
        let result = audit_proposal_checked(
            &invalid,
            &RetroGenerationContext {
                depth: 1,
                max_proposals: 1,
            },
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn rejects_duplicate_or_mixed_abstain_output() {
        let mut duplicate = proposal("candidate-1");
        duplicate.provenance.source_rank = 1;
        let decision = RetroGeneratorDecision {
            proposals: vec![proposal("candidate-1"), duplicate],
            abstained: false,
        };
        assert!(
            validate_decision(
                "CCO",
                &RetroGenerationContext {
                    depth: 0,
                    max_proposals: 0,
                },
                &decision
            )
            .is_err()
        );

        let decision = RetroGeneratorDecision {
            proposals: vec![proposal("candidate-1")],
            abstained: true,
        };
        assert!(validate_decision("CCO", &RetroGenerationContext::default(), &decision).is_err());
    }

    #[test]
    fn rejects_untraceable_or_invalid_confidence() {
        let mut invalid = proposal("candidate-1");
        invalid.provenance.generator_id.clear();
        assert!(
            validate_decision(
                "CCO",
                &RetroGenerationContext::default(),
                &RetroGeneratorDecision {
                    proposals: vec![invalid],
                    abstained: false,
                }
            )
            .is_err()
        );

        let mut invalid = proposal("candidate-2");
        invalid.provenance.model_confidence = Some(f64::NAN);
        assert!(
            validate_decision(
                "CCO",
                &RetroGenerationContext::default(),
                &RetroGeneratorDecision {
                    proposals: vec![invalid],
                    abstained: false,
                }
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_ambiguous_atom_mapping() {
        let mut mapped = proposal("candidate-1");
        mapped.atom_mapping = Some(vec![
            AtomMapEntry {
                map_num: 1,
                target_atom_index: 0,
                precursor_index: 0,
                precursor_atom_index: 0,
            },
            AtomMapEntry {
                map_num: 1,
                target_atom_index: 1,
                precursor_index: 1,
                precursor_atom_index: 0,
            },
        ]);
        assert!(
            validate_decision(
                "CCO",
                &RetroGenerationContext::default(),
                &RetroGeneratorDecision {
                    proposals: vec![mapped],
                    abstained: false,
                }
            )
            .is_err()
        );
    }

    #[test]
    fn chemistry_validation_checks_smiles_and_mapping_indices() {
        let mut invalid = proposal("candidate-1");
        invalid.atom_mapping = Some(vec![AtomMapEntry {
            map_num: 1,
            target_atom_index: 99,
            precursor_index: 0,
            precursor_atom_index: 0,
        }]);
        let decision = RetroGeneratorDecision {
            proposals: vec![invalid],
            abstained: false,
        };
        assert!(validate_chemistry("CCO", &decision).is_err());

        let mut invalid = proposal("candidate-2");
        invalid.precursors = vec!["not a smiles".to_owned()];
        let decision = RetroGeneratorDecision {
            proposals: vec![invalid],
            abstained: false,
        };
        assert!(validate_chemistry("CCO", &decision).is_err());
    }

    #[test]
    fn rule_based_adapter_round_trips_existing_one_step_candidates() {
        let rules = crate::chem_env::default_rules();
        let proposal_context = crate::candidate::CandidateProposalContext::new(&rules, false);
        let proposal_config = crate::candidate::ProposalConfig::default();
        let generator = RuleBasedRetroGenerator::new(&proposal_context, &proposal_config);
        let decision = generator
            .propose_checked("CCO", &RetroGenerationContext::default())
            .unwrap();

        assert!(!decision.abstained);
        for proposal in decision.proposals {
            assert!(!proposal.candidate_id.is_empty());
            assert_eq!(proposal.target, "CCO");
            assert!(!proposal.precursors.is_empty());
            assert_eq!(proposal.provenance.generator_id, "renkin-rule-proposer");
        }
    }

    #[test]
    fn checked_entry_point_blocks_invalid_model_output() {
        let generator = FixtureGenerator {
            decision: RetroGeneratorDecision {
                proposals: vec![proposal("candidate-1"), proposal("candidate-1")],
                abstained: false,
            },
        };
        let error = generator
            .propose_checked("CCO", &RetroGenerationContext::default())
            .unwrap_err();
        assert!(error.contains("duplicate candidate_id"));
    }
}
