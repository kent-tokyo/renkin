//! Atom-map diagnostics for an already-normalized route audit.
//!
//! This module deliberately reports a separate receipt instead of adding new
//! gating findings. Atom maps are source-tool evidence: a missing, malformed,
//! or locally inconsistent map must be visible to a caller, but must not
//! rewrite an established route `pass`/`fail`/`partial` verdict. In
//! particular, map numbers are local to a reaction unless a concrete
//! producer/consumer boundary can be identified from the normalized tree.

use std::collections::{BTreeMap, BTreeSet};

use chematic::rxn::parse_reaction;
use serde::Serialize;

use crate::bridge::forward::{ForwardNotEvaluableReason, declared_smirks};
use crate::bridge::route_graph::ReactionEvidence;
use crate::chem_env::{RetroRule, clear_atom_maps, to_canonical};

/// Verdict for one atom-map diagnostic. This is deliberately independent of
/// [`crate::bridge::audit::CheckStatus`]: it is evidence about mapping, not a
/// route acceptance decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AtomMappingStatus {
    Valid,
    Invalid,
    NotEvaluable,
}

/// Reason codes for a step-local atom-map receipt. `Missing*` and
/// `Unsupported*` mean the receipt could not be evaluated; duplicate or
/// orphan map labels mean the supplied reaction record is internally
/// inconsistent. None of these codes changes the audit's existing status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AtomMappingReason {
    MissingReactionRepresentation,
    MissingAtomMapping,
    UnsupportedReactionFormat,
    DuplicateReactantAtomMap,
    DuplicateProductAtomMap,
    ProductAtomMapWithoutReactant,
}

/// Reason codes for an edge between a deeper reaction (the forward producer)
/// and its parent reaction (the forward consumer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProducerConsumerMappingReason {
    ProducerMapUnavailable,
    ConsumerMapUnavailable,
    ProducerProductNotIdentified,
    ConsumerReactantNotIdentified,
    ProducerConsumerMapMismatch,
}

/// Evidence for one concrete forward producer -> consumer boundary. The
/// producer is the current, deeper audited step; `consumer_step_index` names
/// its parent in preorder `AuditReport.steps` order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProducerConsumerMappingReceipt {
    pub consumer_step_index: usize,
    pub status: AtomMappingStatus,
    pub reasons: Vec<ProducerConsumerMappingReason>,
}

/// Step-local map evidence plus, for a non-root decomposition, the optional
/// route-edge check connecting the step to its parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AtomMappingReceipt {
    pub status: AtomMappingStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reactant_map_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_map_count: Option<usize>,
    pub reasons: Vec<AtomMappingReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub producer_consumer: Option<ProducerConsumerMappingReceipt>,
}

#[derive(Debug, Clone)]
struct MappedComponent {
    unmapped_canonical: String,
    mapped_canonical: String,
}

/// Internal companion to [`AtomMappingReceipt`]. Components are retained only
/// while building the report, never serialized as extra source-tool data.
#[derive(Debug, Clone)]
pub(crate) struct AtomMappingInspection {
    pub receipt: AtomMappingReceipt,
    reactants: Option<Vec<MappedComponent>>,
    products: Option<Vec<MappedComponent>>,
}

fn unavailable(reason: AtomMappingReason) -> AtomMappingInspection {
    AtomMappingInspection {
        receipt: AtomMappingReceipt {
            status: AtomMappingStatus::NotEvaluable,
            reactant_map_count: None,
            product_map_count: None,
            reasons: vec![reason],
            producer_consumer: None,
        },
        reactants: None,
        products: None,
    }
}

fn unavailable_from_forward_reason(reason: ForwardNotEvaluableReason) -> AtomMappingReason {
    match reason {
        ForwardNotEvaluableReason::MissingReactionRepresentation => {
            AtomMappingReason::MissingReactionRepresentation
        }
        ForwardNotEvaluableReason::MissingAtomMapping => AtomMappingReason::MissingAtomMapping,
        ForwardNotEvaluableReason::UnsupportedReactionFormat
        | ForwardNotEvaluableReason::UnsupportedTemplateSyntax
        | ForwardNotEvaluableReason::ReactionApplicationError
        | ForwardNotEvaluableReason::AmbiguousExpectedProduct => {
            AtomMappingReason::UnsupportedReactionFormat
        }
    }
}

fn components_and_maps(
    molecules: &[crate::chem_env::Molecule],
) -> (Vec<MappedComponent>, BTreeMap<u16, usize>, BTreeSet<u16>) {
    let mut components = Vec::with_capacity(molecules.len());
    let mut counts = BTreeMap::new();
    let mut maps = BTreeSet::new();
    for molecule in molecules {
        components.push(MappedComponent {
            unmapped_canonical: to_canonical(&clear_atom_maps(molecule)),
            mapped_canonical: to_canonical(molecule),
        });
        for (_, atom) in molecule.atoms() {
            if let Some(map) = atom.atom_map {
                *counts.entry(map).or_insert(0) += 1;
                maps.insert(map);
            }
        }
    }
    (components, counts, maps)
}

/// Inspect the reaction representation already claimed by one route step.
/// It never searches alternate templates or fabricates maps.
pub(crate) fn inspect_step_mapping(
    evidence: Option<&ReactionEvidence>,
    rules_by_template_id: Option<&std::collections::HashMap<String, &RetroRule>>,
) -> AtomMappingInspection {
    let Some(evidence) = evidence else {
        return unavailable(AtomMappingReason::MissingReactionRepresentation);
    };
    let (smirks, _, _) = match declared_smirks(evidence, rules_by_template_id) {
        Ok(value) => value,
        Err(reason) => return unavailable(unavailable_from_forward_reason(reason)),
    };
    let reaction = match parse_reaction(smirks) {
        Ok(reaction) => reaction,
        Err(_) => return unavailable(AtomMappingReason::UnsupportedReactionFormat),
    };
    let (reactants, reactant_counts, reactant_maps) = components_and_maps(&reaction.reactants);
    let (products, product_counts, product_maps) = components_and_maps(&reaction.products);
    let reactant_map_count = reactant_counts.values().sum();
    let product_map_count = product_counts.values().sum();
    if reactant_map_count == 0 && product_map_count == 0 {
        return unavailable(AtomMappingReason::MissingAtomMapping);
    }

    let mut reasons = Vec::new();
    if reactant_counts.values().any(|&count| count > 1) {
        reasons.push(AtomMappingReason::DuplicateReactantAtomMap);
    }
    if product_counts.values().any(|&count| count > 1) {
        reasons.push(AtomMappingReason::DuplicateProductAtomMap);
    }
    if !product_maps.is_subset(&reactant_maps) {
        reasons.push(AtomMappingReason::ProductAtomMapWithoutReactant);
    }
    let status = if reasons.is_empty() {
        AtomMappingStatus::Valid
    } else {
        AtomMappingStatus::Invalid
    };
    AtomMappingInspection {
        receipt: AtomMappingReceipt {
            status,
            reactant_map_count: Some(reactant_map_count),
            product_map_count: Some(product_map_count),
            reasons,
            producer_consumer: None,
        },
        reactants: Some(reactants),
        products: Some(products),
    }
}

/// Attach an evidence-only producer/consumer receipt to `producer`. The
/// normalized route already proves the unmapped structural edge; this checks
/// only whether the two source reaction records preserve the same mapped form
/// of that shared intermediate.
pub(crate) fn attach_producer_consumer_receipt(
    producer: &mut AtomMappingInspection,
    consumer: &AtomMappingInspection,
    consumer_step_index: usize,
    shared_target: &str,
) {
    let receipt = match producer.products.as_ref() {
        None => ProducerConsumerMappingReceipt {
            consumer_step_index,
            status: AtomMappingStatus::NotEvaluable,
            reasons: vec![ProducerConsumerMappingReason::ProducerMapUnavailable],
        },
        Some(_) if consumer.reactants.is_none() => ProducerConsumerMappingReceipt {
            consumer_step_index,
            status: AtomMappingStatus::NotEvaluable,
            reasons: vec![ProducerConsumerMappingReason::ConsumerMapUnavailable],
        },
        Some(products) => {
            let producer_matches: Vec<_> = products
                .iter()
                .filter(|component| component.unmapped_canonical == shared_target)
                .collect();
            if producer_matches.len() != 1 {
                ProducerConsumerMappingReceipt {
                    consumer_step_index,
                    status: AtomMappingStatus::NotEvaluable,
                    reasons: vec![ProducerConsumerMappingReason::ProducerProductNotIdentified],
                }
            } else {
                let consumer_matches: Vec<_> = consumer
                    .reactants
                    .as_ref()
                    .expect("checked above")
                    .iter()
                    .filter(|component| component.unmapped_canonical == shared_target)
                    .collect();
                if consumer_matches.len() != 1 {
                    ProducerConsumerMappingReceipt {
                        consumer_step_index,
                        status: AtomMappingStatus::NotEvaluable,
                        reasons: vec![ProducerConsumerMappingReason::ConsumerReactantNotIdentified],
                    }
                } else if producer_matches[0].mapped_canonical
                    == consumer_matches[0].mapped_canonical
                {
                    ProducerConsumerMappingReceipt {
                        consumer_step_index,
                        status: AtomMappingStatus::Valid,
                        reasons: vec![],
                    }
                } else {
                    ProducerConsumerMappingReceipt {
                        consumer_step_index,
                        status: AtomMappingStatus::Invalid,
                        reasons: vec![ProducerConsumerMappingReason::ProducerConsumerMapMismatch],
                    }
                }
            }
        }
    };
    producer.receipt.producer_consumer = Some(receipt);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::route_graph::ReactionEvidence;

    fn evidence(smirks: &str) -> ReactionEvidence {
        ReactionEvidence::SynPlannerReaction {
            smiles: smirks.into(),
            rule_provenance: None,
        }
    }

    #[test]
    fn valid_mapped_step_receipt_is_separate_from_route_status() {
        let inspection =
            inspect_step_mapping(Some(&evidence("[CH3:1][OH:2]>>[CH3:1].[OH:2]")), None);
        assert_eq!(inspection.receipt.status, AtomMappingStatus::Valid);
        assert_eq!(inspection.receipt.reactant_map_count, Some(2));
        assert_eq!(inspection.receipt.product_map_count, Some(2));
        assert!(inspection.receipt.reasons.is_empty());
    }

    #[test]
    fn duplicate_and_orphan_maps_are_invalid_without_being_repaired() {
        let inspection =
            inspect_step_mapping(Some(&evidence("[CH3:1][OH:1]>>[CH3:2].[OH:1]")), None);
        assert_eq!(inspection.receipt.status, AtomMappingStatus::Invalid);
        assert_eq!(
            inspection.receipt.reasons,
            vec![
                AtomMappingReason::DuplicateReactantAtomMap,
                AtomMappingReason::ProductAtomMapWithoutReactant,
            ]
        );
    }

    #[test]
    fn boundary_compares_mapped_forms_only_after_structural_identity() {
        let mut producer =
            inspect_step_mapping(Some(&evidence("[CH3:1][OH:2]>>[CH3:1][OH:2]")), None);
        let consumer = inspect_step_mapping(Some(&evidence("[CH3:1][OH:2]>>[CH3:1].[OH:2]")), None);
        let shared_target =
            to_canonical(&crate::chem_env::mol_from_smiles("CO").expect("fixture SMILES parses"));
        attach_producer_consumer_receipt(&mut producer, &consumer, 0, &shared_target);
        assert_eq!(
            producer.receipt.producer_consumer,
            Some(ProducerConsumerMappingReceipt {
                consumer_step_index: 0,
                status: AtomMappingStatus::Valid,
                reasons: vec![],
            })
        );
    }
}
