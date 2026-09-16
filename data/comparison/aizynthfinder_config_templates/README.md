# AiZynthFinder formal configuration templates

`compare_run.py` compares the mounted AiZynthFinder configuration byte-for-byte
with the template selected by `--comparison-mode`. This makes the effective
search budget and route-output limit part of the run identity.

After downloading the ignored public model/stock bundle, provision it with:

```bash
cp data/comparison/aizynthfinder_config_templates/config.yml \
  data/comparison/aizynthfinder_public_data/config.yml
cp data/comparison/aizynthfinder_config_templates/config_shared_stock.yml \
  data/comparison/aizynthfinder_public_data/config_shared_stock.yml
```

For a separately registered Phase 55-style 31-second, rank-1 shared-stock
comparison, provision the dedicated filename without overwriting the standard
120-second/top-5 configuration:

```bash
cp data/comparison/aizynthfinder_config_templates/config_phase55_31s_rank1_shared_stock.yml \
  data/comparison/aizynthfinder_public_data/config_phase55_31s_rank1_shared_stock.yml
```

Pass both `--aizynthfinder-config-filename
config_phase55_31s_rank1_shared_stock.yml` and
`--aizynthfinder-config-template
data/comparison/aizynthfinder_config_templates/config_phase55_31s_rank1_shared_stock.yml`
to `compare_run.py`. The adapter rejects a mounted file that does not
byte-match the selected template.

The templates explicitly set MCTS depth (`max_transforms: 5`), iteration cap
(100), search deadline (120 seconds), `return_first: false`, and top-5 route
extraction. Do not edit the mounted copies for a formal run; make a tracked
template change, review it, then provision it again.
