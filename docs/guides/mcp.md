# MCP server

`renkin-mcp` is a local JSON-RPC server over standard input/output. It lets an
agent plan, inspect, and audit with the same bounded Rust core as the CLI. It
does not fetch remote routes, upload private inputs, or make model calls.

## Start it

```bash
cargo run --release --bin renkin-mcp
```

Use an MCP client that supports either the legacy `2024-11-05` protocol or the
modern `2026-07-28` protocol. Modern clients should call `initialize`, then
`tools/list`; tool schemas and effective limits are advertised rather than
being inferred from this page.

## Tools

| Tool | Purpose |
| --- | --- |
| `find_routes` | Search bounded retrosynthetic routes from a target SMILES. |
| `validate_route` | Audit route structure, stock coverage, and forward basis. |
| `explain_route` | Render an existing route and its findings for review. |
| `find_pareto_routes` | Return routes across explicit cost/quality trade-offs. |
| `plan_with_constraints` | Plan with explicit stock, cost, element, and family constraints. |
| `estimate_diversity` | Summarize route-set diversity without claiming experimental independence. |
| `diagnose_failure` | Report search loss signals for an unsolved target. |

`tools/list` is authoritative for each argument schema, required fields, and
stable/experimental status. Clients must reject unknown parameters rather than
assuming they are ignored.

## Limits and receipts

Every MCP surface exposes a machine-readable capability/limit contract. It
includes accepted tool names, search and audit limits, the local-only network
stance, and cancellation behavior. Validate it at session setup and pin it in
an agent run record.

Successful modern `tools/call` responses include a redacted audit receipt when
applicable. A receipt records the tool, normalized argument hash, outcome,
and parent linkage without embedding private request bodies. It is evidence of
this local invocation, not proof of experimental synthesis success.

## Failure behavior

The server validates JSON-RPC before tool dispatch:

- malformed JSON is a `-32700` parse error;
- malformed envelopes or unsupported protocol versions are `-32600` invalid
  requests;
- unknown methods are `-32601`;
- notifications produce no response;
- an invalid request never poisons the following frame.

Tool errors are structured results. Resource exhaustion, parse rejection,
deadline expiry, and route-validation failure remain distinct; none is an
empty successful route list. MCP uses stdio only, so reserve stdout for
protocol frames and send human diagnostics to stderr.

## Safe agent workflow

1. Discover tools and capability limits.
2. Submit bounded local work with explicit stock and policy where relevant.
3. Keep the route result, manifest, and receipt together.
4. Treat `partial` and `not_evaluable` as missing evidence, not success.

For audit policy and manifest fields, read the [audit reproducibility
contract](audit-reproducibility-contract.md). For browser limits, read the
[WASM API](../api/wasm.md).
