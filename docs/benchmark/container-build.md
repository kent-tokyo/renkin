# Bounded planner runtimes

The formal comparison requires all four arms to run under the same Linux
resource envelope. AiZynthFinder already has a pinned container recipe in
`docker/aizynthfinder.Dockerfile`. The two Python planner recipes are:

```sh
docker build --platform linux/arm64 \
  -f docker/syntheseus-benchmark.Dockerfile \
  -t renkin-bench/syntheseus:0.8.0 .
docker build --platform linux/arm64 \
  -f docker/synplanner-benchmark.Dockerfile \
  -t renkin-bench/synplanner:1.6.0 .
```

Before a formal run, record each image digest and `requirements-lock.txt`
hash in the registry/run manifest. The image tag alone is not provenance.

After the build, verify the immutable local image IDs with:

```sh
python3 scripts/verify_four_tool_images.py \
  benchmarks/four_tool/configuration_registry.json \
  --output <run-dir>/image-identities.json
```

The external runner accepts `--container-image`. It mounts the checked-out
repository read-only at `/repo`, the per-run artifact directory at
`/artifacts`, disables network access, and applies `--cpus 8 --memory 6g
--memory-swap 6g`. All model/checkpoint paths used by this mode must be under
the checked-out repository so the path translation is deterministic.

On macOS, host-native runs are useful for feasibility only: the platform does
not accept the finite `RLIMIT_AS` used by the fallback resource helper. They
must not be reported as equivalent to the Linux bounded formal arms.
