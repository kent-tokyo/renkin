# MCP protocol fixtures

## `2024-11-05/` — legacy compatibility oracle

`legacy_transcript_input.jsonl` / `legacy_transcript_output.jsonl` /
`legacy_transcript_stderr.txt` were captured by running the **unmodified**
`renkin-mcp` binary built from `origin/master` at commit `d90279a` (the exact
commit this worktree branched from, before any refactor in this PR), via:

```bash
cargo build --release --bin renkin-mcp
target/release/renkin-mcp < tests/fixtures/mcp/2024-11-05/legacy_transcript_input.jsonl \
  > tests/fixtures/mcp/2024-11-05/legacy_transcript_output.jsonl \
  2> tests/fixtures/mcp/2024-11-05/legacy_transcript_stderr.txt
```

This is the falsifiable oracle for legacy behavior at the original refactor
boundary: the post-refactor binary must produce structurally-equal JSON for
the same input except for explicitly documented additions made on master
after capture (currently the `find_routes` coverage-search schema fields).
Top-level key order is allowed to differ because `serde_json::Value` does not
guarantee insertion order. The fixture intentionally captures the
pre-existing "unknown tool name silently falls back to `find_routes`" bug
(request id 6) — that bug is preserved for legacy clients in this PR (fixing
it would be a legacy behavior change, out of scope) and is fixed only in the
modern era's dispatch path (see `docs/guides/mcp.md`).

The 2024-11-05 schema itself is not vendored here: it is frozen upstream and
RENKIN's own pre-existing implementation (predating this PR) is the reference
implementation this transcript pins down.

## `2026-07-28-rc/` — modern era source of truth

`schema.ts` / `schema.json` / `examples/**/*.json` were downloaded verbatim
from the official spec repository at a pinned commit — **not** inferred from
blog posts, SDKs, or this task's own prose description of the wire format.

```
MCP_SPEC_REPO=https://github.com/modelcontextprotocol/modelcontextprotocol
MCP_SPEC_REVISION=2026-07-28-RC   (schema/draft at this commit; LATEST_PROTOCOL_VERSION="2026-07-28")
MCP_SPEC_COMMIT=7634684382c3d14cf7e9f14073fe40a2d8ace3fa
MCP_SPEC_COMMIT_DATE=2026-07-23T23:49:30Z
```

SHA-256 (recomputed at commit-pinned URLs, matches an earlier fetch from the
moving `main` ref taken the same day — no drift observed):

```
c56f0ad2395f9f7109a903a304344a61c65555cb0b2d28c1635cc32497221c87  schema.ts
9281c4890630e2d1e61792fa23b4084c4ea360cd58519610cd050545ab7b8708  schema.json
```

License (verbatim from the upstream repo's `LICENSE` at the pinned commit,
see `UPSTREAM-LICENSE` in this directory): the MCP project is mid-transition
from MIT to Apache-2.0. New spec/code contributions are Apache-2.0;
contributions from authors who have not consented to relicensing remain
MIT; documentation (excluding specs) is CC-BY-4.0. This is **not** simply
"MIT" or "CC-BY-SA-4.0" — do not summarize it as either.

### Conformance suite

`modelcontextprotocol/conformance` was checked at commit `a865118206d4d8cc8dbc5f5201607839281d0c3b`
(2026-07-23). At that commit the suite is a **client** conformance framework:
it spins up an HTTP test server and drives a client implementation against
it. Its `test:server` mode targets `--server-url http://.../mcp` (Streamable
HTTP only). No stdio-server scenario exists at this commit, so it cannot be
run against `renkin-mcp` (stdio-only, per this PR's scope). This project's
own tests (`tests/mcp_*`) are therefore the only conformance evidence for the
2026-07-28 stdio subset RENKIN implements — see the PR body for the exact
phrasing used, which does not claim official conformance.

### Final-spec delta check

Completed 2026-09-05 against the official `2026-07-28` GA tag:

```
742750af0bb8c716e7030c4977c992b55d1adc4407e9e66997db5846baedc2cd  schema.ts
ef70b61f99b6d2e5e3b46863822eab08dff6a45bedc7a08914e0e5b133f40203  schema.json
```

The RC-to-GA semantic delta is confined to `subscriptions/listen`: the
`SubscriptionsListenResultMeta` type was renamed to
`SubscriptionsListenResultMetaObject`, a graceful-close result response was
added, and TypeScript documentation links moved from `/draft/` to
`/2026-07-28/`. RENKIN does not implement subscriptions, and no
stdio/tool-schema/error-code shape used by this server changed. The upstream
license file is byte-identical at GA (SHA-256
`0382b0057770ca05e9c350a50aa3b1c1fea84da0bc81d723bf00b9aa841be58a`).
