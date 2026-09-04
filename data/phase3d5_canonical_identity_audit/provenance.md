# Phase 3D.5 -- provenance snapshot at audit start

Recorded before any audit work, per the user's explicit Step 0 instruction,
so the audit's conclusions are reproducible against a fixed baseline.

| item | value |
|---|---|
| renkin git HEAD (at audit start) | `d1d358cdc3120290c4012b4665871ef973aebaa8` (branch `feat/reranker-real-data-gate-101`) |
| `renkin-pool-gen` binary SHA-256 | `sha256:ff35e90628bae18d2df233305a5a2d0ce3cc67dcb28218666f7db578151a288d` |
| `Cargo.lock` SHA-256 | `sha256:ada045bd1b65319d8ede433417d50991b5f7866464c522eedca07de7a6775174` |
| chematic version (pinned) | 0.11.0 |

## Full pools (Phase 3D, `data/phase3d_full_pool/`)

| file | SHA-256 |
|---|---|
| `pool_train_full.jsonl` (candidate_jsonl_sha256) | `sha256:43d0f5c97a52314262e3aa421bffe7b5c1982b92e9d75d444c17acaaea2183b4` |
| `groups_train_full.jsonl` (target_group_index_sha256) | `sha256:5c3d135703daaeecf01ee2452e551ea2637fa597a4f8f3bba7dd9a90cc026fc0` |
| `pool_val_full.jsonl` (candidate_jsonl_sha256) | `sha256:2770047e6103ffee9d63b9122996739cb8f0fa17b1fc42312eed742ee38fc1e1` |
| `groups_val_full.jsonl` (target_group_index_sha256) | `sha256:bb93a9b46c7eafd6b9dc2f138e020f2e9407a6e11bf73e8c53405b9cb26fe6ca` |

Full pools are kept on disk, not deleted; not committed to git (large binaries,
already covered by `.gitignore`). Whether regeneration is needed is decided
after this audit, per the user's CASE A/B/C decision rule.

## Source label/split-manifest files (gitignored, regenerable per Phase 3A's provenance)

| file | SHA-256 |
|---|---|
| `reranker_labels_uspto50k_train.jsonl` | `sha256:954d0a661fa6aeef0452d9f7f5b687fff9ac3142f9cf4b82d256ff2f60ba6f47` |
| `reranker_labels_uspto50k_val.jsonl` | `sha256:a623d73932293899de1930bad535186acaf002f8ef447ea88bedd4fe7667b342` |
| `reranker_labels_uspto50k_test.jsonl` | `sha256:1dfb4fe78b73d769572649a67d6b41c32e0b1e04810f999cae7f14bf3c774417` |
| `reranker_split_manifest.jsonl` | `sha256:c55ff1dfd04eadcd0b009fbed934ce1537dac6d56fb40afd1d53c7196292905d` |
| `reranker_groups_uspto50k_train.jsonl` | `sha256:30d97956da924f847658dd98553536ab1b44c7e0d2db2890059286f45a2bbdee` |
| `reranker_groups_uspto50k_val.jsonl` | `sha256:5b201bf3bb8428e135aa24949db142c5a144aaf1674941a4afb15c91dbedd0bf` |
| `reranker_groups_uspto50k_test.jsonl` | `sha256:45d583f7acb2fa0fe79358f5c545b15eb86bd02e55faaac40165861a0338a82e` |

## Prior decontamination counts (for Step 6 comparison)

Verified against `data/phase3a_reranker_ground_truth_audit/round2_split_hygiene.md`
Section B/C (not re-derived from memory):

| | raw reactions | unique products | overlapping benchmark identities | rows removed |
|---|---|---|---|---|
| train | 40,008 | 39,736 | 68 | 81 |
| val | 5,001 | 4,993 | 7 | 7 |

All via canonical-`target_id`-string matching against the 4,903 quarantined
benchmark identities (every quarantined identity checked against every
train/val product, not sampled). A further, separate cross-split dedup step
(train wins) removed 63 more val rows (62 distinct target_ids) so no
`target_id` appears in both train and val. Step 6 of this audit re-checks
whether any additional benchmark overlap or cross-split overlap exists that
canonical-string comparison would have missed (i.e. two structurally
identical molecules whose canonical strings currently differ, of the kind
Phase 3C/3D just found for 123 targets).
