# Security Policy

## Supported Versions

Security fixes are provided for the latest released version only.

| Version | Supported |
|---|---|
| latest | ✅ |
| older | ❌ |

## Reporting a Vulnerability

**Do not open a public GitHub issue.**

Use [GitHub Private vulnerability reporting](https://github.com/kent-tokyo/renkin/security/advisories/new) from the Security tab.

Please include:
- Affected version or commit
- OS and environment
- Steps to reproduce
- Expected vs. actual behavior
- Proof-of-concept input, crash log, or command output
- Potential impact (if known)

## Response Expectations

I will try to acknowledge valid reports within **7 days**.

After triage:
- Confirm reproducibility
- Assess severity and affected versions
- Prepare a fix privately when appropriate
- Publish a security advisory for issues affecting released versions

## Scope

In scope:
- Memory safety issues
- Panics or crashes triggered by untrusted input (malformed SMILES, reaction templates, route JSON)
- Denial-of-service from malformed input
- Unsafe file handling
- Dependency vulnerabilities
- Secret exposure in workflows or release automation

Out of scope:
- General bugs without security impact
- Inaccurate retrosynthesis predictions
- Performance issues without DoS impact
- Reports against unsupported versions

## Surface inventory and release contract

RENKIN treats target SMILES, route JSON, reaction templates, stock files,
evidence files, and MCP messages as attacker-controlled unless a local bundle
has been explicitly identified and hash-verified. The supported surfaces have
different trust boundaries but share the same security cases:

| Surface | Untrusted boundary | Primary controls | Owner |
|---|---|---|---|
| CLI | arguments, route/template/stock paths and contents | typed validation, bounded search, fail-closed file checks | maintainers |
| Python | PyO3 arguments and serialized route data | parser errors, no panic across the binding, bounded options | maintainers |
| WASM | browser-provided strings and JSON | local-only execution, explicit limits, structured errors | maintainers |
| MCP | JSON-RPC lines, ids, methods, params | protocol validation, bounded request work, stdout discipline | maintainers |
| Library | caller-provided molecules, rules, and policies | `Result` boundaries, deterministic validation, no new `unsafe` | maintainers |
| CI/release | workflows, dependencies, generated artifacts | pinned inputs, hash/provenance checks, release gates | maintainers |

The comparison run manifest is the machine-readable S0 record. It includes
`security_contract.version`, the threat-case list, the resource budget used for
the run, and release blockers. Every threat case uses a stable
`security_case_id`, severity, affected surfaces, and a blocker condition. A
manifest with changed input hashes, missing bounds, or an unclassified process
failure is not release evidence.

Minimum release blockers:

- no newly reachable panic, memory-unsafe code, or unbounded resource path;
- input and bundle hashes are captured and unchanged for the whole run;
- malformed input and protocol errors are classified rather than silently
  discarded;
- timeout, budget exhaustion, parse rejection, and validation failure remain
  distinct outcomes;
- no local username, unredacted home path, secret, or stack trace is emitted
  into a shareable manifest or error response.

The MCP transport also has a fail-closed protocol rule: malformed JSON is a
`-32700` Parse error, a non-object request, missing/non-string method, or
non-scalar request ID is a `-32600` Invalid Request, and an unknown method is a
`-32601` Method not found. These errors are structured JSON-RPC responses and
never echo the rejected input.

Within `tools/call`, `params` and `arguments` must be JSON objects, the tool
name must be present in the advertised allowlist, and unknown names are
rejected instead of falling through to the default `find_routes` handler.

MCP request lines are capped at 1 MiB before JSON parsing. An oversized line is
drained through its newline and rejected as `-32600 resource_exhausted: request
too large`, so it
cannot cause an unbounded allocation or shift subsequent request framing.

The shared search entry point rejects oversized target SMILES and excessive
depth, route count, beam width, or candidate-trace caps before chemical parsing
or expansion. The resulting `resource_exhausted` error is propagated through
the library, CLI, Python, WASM, and MCP adapters rather than being treated as
an empty search result.

The streaming stock importer applies independent bounds of 64MiB total input,
64KiB per line, and one million data rows. It rejects invalid UTF-8 and
resource exhaustion before producing a stock manifest.

Template metadata sidecars are likewise restricted to regular UTF-8 files of
at most 64MiB, with the bound enforced while reading (including files that
grow after an initial metadata check) before JSON parsing begins.

`audit-route` route exports are capped at 64MiB both before and after gzip
decompression. Non-regular files, invalid UTF-8, and decompressed expansion
past the cap are rejected before route normalization.

CLI stock, private-stock, stock-policy, vendor-index, and vendor-match text
files use the same 64MiB regular-file/UTF-8 preflight before CSV, TSV, or JSON
parsing.

Python forward APIs cap a call at 32 reactants and 64KiB of reactant text,
limit returned predictions to 1,000, and cap route validation input at 64MiB
and 10,000 steps. These checks return `resource_exhausted` before chemical
prediction or route-step iteration.

In-memory Python and WASM audit APIs cap route and stock text at 64MiB and
stock lines at 64KiB before JSON parsing or stock scanning. File-backed CLI
audit inputs use the equivalent bounded reader.

Route and template-metadata JSON are also preflighted at 256 nesting levels and
one million structural delimiters before deserialization, with excess input
rejected as `resource_exhausted`.

MCP request lines apply the same structural preflight before JSON-RPC dispatch;
requests over the structural budget return a parse error and do not reach a
tool handler.

User-supplied template, metadata, stock-import, and bounded CLI text paths are
checked with `symlink_metadata` and reject symlinks before opening the target.
Evidence-match, template diagnostics, ring-context, template utility, and
constraint JSON paths use the same bounded reader; malformed constraint JSON is
reported as an input error rather than silently ignored.

## Disclosure

Please allow reasonable time for a fix before public disclosure.
Reporters will be credited in the advisory unless they prefer anonymity.
