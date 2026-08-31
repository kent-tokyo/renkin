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

The MCP envelope also requires the exact JSON-RPC version string `"2.0"` before
method dispatch. Missing, non-string, or unsupported protocol versions are
rejected as `-32600 Invalid Request` without reaching a tool handler.

Notifications are identified by an omitted `id` field, independent of the
method name. They may execute their method but never produce a response; an
explicit JSON `null` id remains a request id and receives a correlated response.

Within `tools/call`, `params` and `arguments` must be JSON objects, the tool
name must be present in the advertised allowlist, and unknown names are
rejected instead of falling through to the default `find_routes` handler.
Tool arguments are checked against the advertised per-tool allowlist, so
misspelled or unsupported fields are rejected instead of being ignored and
silently changing the effective request.
The subprocess integration suite exercises these protocol rules through the
real stdio transport, including rejection before dispatch and preservation of
the following frame after an invalid request.
It also covers unknown tool arguments and overflowing search budgets at the
same process boundary, before any chemical search work starts.
The same process-level suite sends multiple rejected requests before a valid
one and verifies that the server remains usable without echoing the rejected
SMILES.
Malformed JSON is also tested with secret-like content to ensure parse errors
remain generic and do not reflect attacker-controlled input.
Numeric search limits are parsed as non-negative integers and checked against
the shared depth and route-count maxima before conversion to native integer
types; malformed values cannot silently default or wrap into a smaller budget.
Optional `find_routes` strings and numeric controls are also type-checked when
present; a supplied boolean, string, or other incompatible JSON value is not
treated as an omitted option.
The same fail-closed rule applies to `plan_with_constraints` route filters,
including numeric thresholds and comma-separated string filters.
Pareto objective specifications are likewise type-checked before route search;
an array or other incompatible value cannot silently select the default
objective set.
Each objective entry must name a supported field and use exactly `min` or
`max`; malformed, unknown, or empty specifications are rejected before search.

MCP request lines are capped at 1 MiB before JSON parsing. An oversized line is
drained through its newline and rejected as `-32600 resource_exhausted: request
too large`, so it
cannot cause an unbounded allocation or shift subsequent request framing.

The shared search entry point rejects oversized target SMILES and excessive
depth, route count, beam width, or candidate-trace caps before chemical parsing
or expansion. The resulting `resource_exhausted` error is propagated through
the library, CLI, Python, WASM, and MCP adapters rather than being treated as
an empty search result.

The dependency policy explicitly allows only the reviewed licenses present in
the locked graph (`MIT`, `Apache-2.0`, `0BSD`, `Unicode-3.0`, and `Zlib`). A
new license therefore fails the local `cargo-deny` gate until reviewed.
The local bans policy rejects wildcard dependencies and keeps duplicate-version
drift visible. The current graph reports only the known transitive `syn` 2/3
split (`wasm-bindgen` versus the serde proc-macro stack); it remains a warning
until an upstream-compatible dependency upgrade can remove it.
The same license, bans, and source checks run in the scheduled security
workflow, so a newly introduced policy violation is a release-blocking CI
failure rather than only a local convention.
The scheduled security workflow also runs the dedicated adversarial regression
suite for the MCP stdio boundary and the manifest security contract, separate
from the general build-and-test workflow.
The gate is also available locally as
`bash scripts/run_security_regressions.sh`; it performs the same dependency,
MCP, and manifest checks without installing project dependencies.
Comparison manifests now reject incomplete or duplicated threat-case metadata
before they can be used as release evidence.
Resume runs validate the persisted manifest before starting benchmark work, so
schema drift or malformed evidence fails fast rather than after an expensive
run.
Manifest loading also enforces a 64MiB bound and rejects symlinks and
non-regular files before deserialization.
Files hashed into comparison provenance use the same boundary before hashing,
covering sample, stock, template, model, and binary inputs.
The limit is enforced during the hash read itself as well as during the
initial metadata check, so file growth during hashing cannot bypass it.
Manifest JSON is also limited to 256 nesting levels before deserialization;
delimiters inside strings do not count toward this limit.
It is additionally limited to 1,000,000 structural JSON tokens, with string
contents excluded from the count.
The benchmark sample corpus applies the same 64MiB regular-file and symlink
boundary, plus a 64KiB per-line limit, before candidate parsing and hashing.
The frozen JSONL sample list is subject to the same limits before row parsing.
Manifest writes use a same-directory temporary file, `fsync`, and atomic
replacement so an interrupted benchmark cannot publish truncated evidence.
Each manifest also receives an independent security-contract snapshot, so
mutating one in post-processing cannot alter later manifests in the process.

The streaming stock importer applies independent bounds of 64MiB total input,
64KiB per line, and one million data rows. It rejects invalid UTF-8 and
resource exhaustion before producing a stock manifest.

Template metadata sidecars are likewise restricted to regular UTF-8 files of
at most 64MiB, with the bound enforced while reading (including files that
grow after an initial metadata check) before JSON parsing begins.

Route JSON supplied to renkin-forward validate through stdin and target lists
used by CLI template coverage use the same 64MiB cap; stdin is bounded during
the read because it has no trustworthy file-size metadata.

The forward CLI integration suite includes an oversized-stdin regression case,
so this boundary is exercised through the real subprocess rather than only
through the shared reader unit test.

The same suite also rejects invalid UTF-8 from stdin before JSON parsing,
keeping encoding failures distinct from malformed JSON.

Reranker model and frequency-table artifacts use the same regular-file,
non-symlink, UTF-8, 64MiB bounded reader before model or JSON parsing.

Binary artifact hashing uses the equivalent regular-file, non-symlink, 64MiB
byte reader, preventing provenance-only paths from bypassing input limits.

Custom template validation/loading and ring-context sidecars use the bounded
text reader before parsing, so these auxiliary search artifacts cannot bypass
the shared size, symlink, regular-file, or UTF-8 checks.

`audit-route` route exports are capped at 64MiB both before and after gzip
decompression. Non-regular files, invalid UTF-8, and decompressed expansion
past the cap are rejected before route normalization.

CLI stock, private-stock, stock-policy, vendor-index, and vendor-match text
files use the same 64MiB regular-file/UTF-8 preflight before CSV, TSV, or JSON
parsing.

CLI stock CSV, stock coverage targets, and CLI/Python building-block price files
use the same fail-closed reader. Missing, unreadable, oversized, symlinked, or
non-UTF-8 inputs now return an error instead of silently becoming empty policy
data.

Stock doctor manifests and forward benchmark verification manifests are also
bounded and checked before JSON deserialization.

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

Forward partner files, benchmark corpora, and coverage templates also use the
descriptor-bounded reader. Coverage template parsing consumes the bounded
content already read from the descriptor, avoiding a second path-based read
after validation.

## Disclosure

Please allow reasonable time for a fix before public disclosure.
Reporters will be credited in the advisory unless they prefer anonymity.
