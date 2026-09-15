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

The templates explicitly set MCTS depth (`max_transforms: 5`), iteration cap
(100), search deadline (120 seconds), `return_first: false`, and top-5 route
extraction. Do not edit the mounted copies for a formal run; make a tracked
template change, review it, then provision it again.
