# MCP Server (`renkin-mcp`)

`renkin-mcp` exposes RENKIN's retrosynthesis tools over the [Model Context
Protocol](https://modelcontextprotocol.io), so AI agents (Claude Desktop,
Claude Code, and other MCP clients) can call them directly.

## Transport

**stdio only.** `renkin-mcp` reads newline-delimited JSON-RPC 2.0 requests
from stdin and writes one JSON-RPC message per line to stdout. Request lines
are capped at 1 MiB and JSON structure is checked against the shared nesting
and token budget before deserialization. Diagnostics go to stderr, never
stdout. Streamable HTTP, OAuth-based authorization, MCP
Apps, and the Tasks extension are not implemented — see
[Non-goals](#non-goals-for-this-release).

## Protocol support matrix

| Protocol revision | Handshake | Status |
|---|---|---|
| `2024-11-05` ("legacy") | `initialize` → `notifications/initialized` → `tools/list` / `tools/call` | Fully supported; stable envelope with additive tool fields |
| `2026-07-28` ("modern") | None — `server/discover` (optional) and per-request `_meta`, negotiated on the first request | Supported for the stdio subset RENKIN uses (see [Conformance](#conformance)) |

A single `renkin-mcp` process serves **either** era per connection, decided
by the first non-notification request it receives:

- First request is `initialize` → the connection is pinned **legacy** for
  its whole lifetime.
- First request is `server/discover`, or an inline `tools/list` /
  `tools/call` carrying valid modern `_meta`, → the connection is pinned
  **modern**.
- A notification alone never pins the connection.
- An ambiguous opening request (e.g. a bare `tools/list` with no `_meta` and
  no prior `initialize`) is rejected without pinning, so a client that
  retries with a valid opening request still works.
- Once pinned, a connection cannot switch eras mid-stream: a legacy
  connection that receives `server/discover` gets `Method not found`; a
  modern connection that receives `initialize` gets `Method not found`
  too (`initialize` has no definition at all in the 2026-07-28 schema).

## Legacy (2024-11-05) clients

No changes from prior RENKIN releases. Register in
`claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "renkin": { "command": "/path/to/renkin-mcp" }
  }
}
```

```
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"legacy-client","version":"1.0"}}}
← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"renkin","version":"1.0.1"}}}

→ {"jsonrpc":"2.0","method":"notifications/initialized"}

→ {"jsonrpc":"2.0","id":2,"method":"tools/list"}
← {"jsonrpc":"2.0","id":2,"result":{"tools":[...]}}

→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"find_routes","arguments":{"smiles":"CC(=O)Nc1ccc(O)cc1"}}}
← {"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"..."}]}}
```

Legacy responses never carry `resultType`, per-request `_meta`,
`supportedVersions`, the modern `_meta.serverInfo` block, or the modern
`tools/list` caching fields (`ttlMs` / `cacheScope`). This is checked by a
regression test against a transcript captured from the binary shipped before
this protocol revision was added
(`tests/fixtures/mcp/2024-11-05/legacy_transcript_output.jsonl`).

Legacy wire envelopes remain compatible, but unsafe dispatch behavior is not
preserved: unknown tool names, misspelled arguments, and out-of-budget numeric
values fail closed before search. Legacy clients receive a normal tool result
with `isError: true`; modern clients receive protocol-level `-32602 Invalid
Params` (see below).

## Modern (2026-07-28) clients

No `initialize` handshake. Every request carries protocol negotiation in
`params._meta`:

```
→ {"jsonrpc":"2.0","id":"d1","method":"server/discover","params":{"_meta":{
    "io.modelcontextprotocol/protocolVersion":"2026-07-28",
    "io.modelcontextprotocol/clientInfo":{"name":"test-client","version":"1.0.0"},
    "io.modelcontextprotocol/clientCapabilities":{}
  }}}
← {"jsonrpc":"2.0","id":"d1","result":{
    "resultType":"complete",
    "supportedVersions":["2026-07-28"],
    "capabilities":{"tools":{}},
    "instructions":"RENKIN provides retrosynthetic route search and route-analysis tools.",
    "ttlMs":3600000,
    "cacheScope":"public",
    "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"renkin","version":"1.0.1"}}
  }}

→ {"jsonrpc":"2.0","id":"t1","method":"tools/list","params":{"_meta":{
    "io.modelcontextprotocol/protocolVersion":"2026-07-28",
    "io.modelcontextprotocol/clientCapabilities":{}
  }}}
← {"jsonrpc":"2.0","id":"t1","result":{
    "resultType":"complete",
    "tools":[...],
    "ttlMs":3600000,
    "cacheScope":"public",
    "_meta":{"io.modelcontextprotocol/serverInfo":{"name":"renkin","version":"1.0.1"}}
  }}
```

`server/discover` is optional — a client can instead send `tools/list` or
`tools/call` directly as its opening request, as long as it carries valid
`_meta`; the connection still pins modern.

### Per-request `_meta`

Every modern request must include, under `params._meta`:

| Key | Required | Notes |
|---|---|---|
| `io.modelcontextprotocol/protocolVersion` | Yes | Must be exactly `"2026-07-28"`; anything else gets `-32022 Unsupported protocol version` with `data: {supported, requested}` |
| `io.modelcontextprotocol/clientCapabilities` | Yes | Must be an object (may be empty) |
| `io.modelcontextprotocol/clientInfo` | No | If present, must have string `name` and `version`; malformed values are rejected |
| `io.modelcontextprotocol/logLevel`, `traceparent`, `tracestate`, `baggage` | No | Accepted but not interpreted — RENKIN does not emit log-level-gated notifications or forward trace context in this release |

Client identity (`clientInfo`) is validated but never used to change
behavior — no authorization or feature branching on client name/version.

### `tools/list` caching hints

Modern `tools/list` responses include `ttlMs: 3600000` (1 hour) and
`cacheScope: "public"`: RENKIN's tool list is static per binary build and
carries no per-user data, so any client or intermediary may cache and share
it. `listChanged` is not advertised (RENKIN doesn't send list-change
notifications). Tool order is fixed by declaration order — not alphabetical
— and is covered by a determinism test, since a caching client may compare
list contents across calls.

### Tool schemas: JSON Schema 2020-12

Modern `inputSchema`/`outputSchema` objects declare `"$schema":
"https://json-schema.org/draft/2020-12/schema"` and `"additionalProperties":
false`, and add numeric bounds RENKIN's own code already documented or
enforces (e.g. `depth`: 1–20, `max_routes`: 1–100, `min_confidence` /
`min_success_probability`: 0–1). Every declared bound is enforced server-side
before the tool handler runs — a modern `tools/call` that violates the
schema never reaches RENKIN's search code; it gets `-32602 Invalid Params`
immediately.

Legacy schemas retain their older JSON shape: they have no `$schema` or
`additionalProperties: false`. The server nevertheless validates legacy
arguments against the same allowlist and numeric bounds before search. Both
eras advertise progressive coverage search and bounded candidate traces on
`find_routes`, and the complete element/building-block/cost/step/confidence/
reaction-family filters on `plan_with_constraints`.

### Structured tool output

`validate_route`, `estimate_diversity`, and `diagnose_failure` return
`structuredContent` (in addition to the existing human-readable `content`
text) in the modern era, validated against a declared `outputSchema`. The
other four tools (`find_routes`, `explain_route`, `find_pareto_routes`,
`plan_with_constraints`) do not yet — their `Route` output is large and
deeply nested; schema-fying it is left to a future PR rather than done
partially here. `estimate_diversity` reports building-block-set diversity and
the deterministic template-disconnection `chemical_idea_cds` proxy as
separate values; the latter is not exact atom-mapped formed-bond CDS.

### Errors

| Condition | Modern behavior |
|---|---|
| Malformed JSON | `-32700 Parse error` (`id: null`) |
| Request line over 1 MiB or JSON structure over budget | `-32600 resource_exhausted` (`id: null`) |
| Missing/wrong `jsonrpc` or `method` | `-32600 Invalid Request` |
| Unknown method | `-32601 Method not found` |
| Unsupported/missing `_meta.protocolVersion` | `-32022` / `-32602` |
| Unknown tool name | `-32602 Invalid Params` — **not** a fallback to `find_routes`, and **not** a tool-level `isError` result |
| Missing/malformed required tool argument | `-32602 Invalid Params` |
| Tool ran but failed for a data/chemistry reason (invalid SMILES, no route found is *not* an error, search internal error) | `isError: true` inside a normal `resultType: "complete"` result |

The unknown-tool-name and missing-argument classification follows the
official schema's own split (`InvalidParamsError`'s doc explicitly lists
"unknown tool name or invalid tool arguments" under protocol-level errors),
not this feature's own early illustrative example, which showed a
missing-argument case as a tool-level error before the schema was checked
against source. The **legacy** era keeps the old tool-level-error behavior
for missing arguments unchanged, since changing it there would be a legacy
behavior break.

## Non-goals for this release

Not implemented, and not advertised as implemented: Streamable HTTP
transport, OAuth/HTTP authorization, MCP Apps, the Tasks extension,
subscriptions, multi-round-trip elicitation, sampling, roots, and
server-to-client requests. RENKIN's tools all complete synchronously in a
single request/response, so Tasks and `input_required` results don't apply
here. `ServerCapabilities.extensions` is omitted from `server/discover`
responses rather than advertised empty.

## Conformance

This implementation's modern wire shapes were checked directly against the
official RC schema (`schema.ts` / `schema.json`) and example fixtures
vendored at `tests/fixtures/mcp/2026-07-28-rc/`, then compared with the
official `2026-07-28` GA tag. The GA delta only renames and extends
`subscriptions/listen` types and updates documentation links; it does not
change the stdio/tool subset implemented here. See the fixture README for
exact hashes and provenance.

The official `modelcontextprotocol/conformance` suite was checked at commit
`a865118206d4d8cc8dbc5f5201607839281d0c3b` (2026-07-23). At that commit it is
a **client**-conformance framework: it spins up its own test server and
drives a client implementation against it, and its server-testing mode
targets Streamable HTTP (`--server-url http://.../mcp`) only. No stdio-server
scenario exists to run RENKIN's `renkin-mcp` against, so this project's own
tests (`tests/mcp_transcript.rs` plus the `#[cfg(test)]` unit tests in
`src/mcp/*`) are the only conformance evidence for now.

Accordingly: RENKIN **supports the MCP 2026-07-28 stdio server subset used
by RENKIN**. It does not claim official conformance, because no official
stdio-server conformance run has been performed.

## Schema pinning and the final-spec delta check

The implementation was originally pinned to RC commit
`7634684382c3d14cf7e9f14073fe40a2d8ace3fa`. The final-spec delta check was
completed against the official `2026-07-28` GA tag on 2026-09-05. The JSON
and TypeScript schemas differ only in `subscriptions/listen` additions and
documentation-link paths, outside RENKIN's declared scope; no implemented
wire shape or error code changed. See `tests/fixtures/mcp/README.md` for the
recorded hashes.

## Troubleshooting

**Client hangs after connecting.** Confirm you're sending newline-delimited
JSON (one object per line, no pretty-printing) — `renkin-mcp` reads
line-by-line and will not process a message until it sees the terminating
`\n`.

**"Unsupported protocol version" from a modern client.** Check
`io.modelcontextprotocol/protocolVersion` is exactly `"2026-07-28"`, not a
different 2025/2026 revision — the error's `data.supported` field lists what
the server accepts.

**A modern client's `tools/call` fails with `-32602` where a legacy client
would have gotten a normal `isError` result.** This is expected for unknown
tool names and missing/malformed required arguments — see
[Errors](#errors) above. It's not a bug; it's the modern era following the
official schema's own error-classification split.

**stderr has diagnostic text mixed into what looks like protocol
output in your client's logs.** `renkin-mcp` never writes anything but
JSON-RPC messages to stdout; if you're seeing mixed output, your MCP client
or supervisor is likely merging separate stdout/stderr streams for display —
check the raw streams independently.
