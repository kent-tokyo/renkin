# Security Policy

## Supported versions

Security fixes are provided for the latest released version of RENKIN only.
Older releases should be upgraded before a report is opened.

## Report privately

Do **not** open a public issue. Use [GitHub Private Vulnerability
Reporting](https://github.com/kent-tokyo/renkin/security/advisories/new).

Include the affected release or commit, operating environment, minimal
reproducer, expected and observed behavior, and any impact you identified.
Do not include secrets, private stock data, or unpublished route data. We aim
to acknowledge valid reports within seven days, then reproduce, assess,
prepare a private fix when appropriate, and publish an advisory for affected
releases.

## Scope

In scope are memory-safety defects, panics or crashes caused by untrusted
input, denial of service, unsafe file handling, dependency vulnerabilities,
and secret exposure in project automation. General planning-quality issues,
performance regressions without a denial-of-service impact, and unsupported
versions are out of scope.

## Security model

Treat target SMILES, route JSON, templates, stock files, evidence sidecars,
and MCP frames as untrusted. RENKIN's public surfaces share these guarantees:

| Surface | Boundary |
| --- | --- |
| CLI, Rust, Python | Validates input and resource limits before chemical work. |
| WASM | Runs locally with declared limits and structured refusals. |
| MCP | Validates JSON-RPC framing and tool arguments before dispatch. |
| Audit and benchmark inputs | Require bounded regular files and preserve hashes/provenance. |

Malformed input, a budget limit, a timeout, and a validation failure remain
distinct outcomes. A rejected request is not silently treated as an empty
route set. Stock identity uses standardized canonical-SMILES exact matching;
substructure matching is never used to promote a leaf to stock.

The project does not make network requests from its core APIs. Private stock
and audit inputs remain local unless an operator explicitly exports them. Error
responses and shareable manifests must not contain secrets, local usernames,
home paths, or attacker-controlled request bodies.

## Verification and disclosure

Run the maintained local security gate before changing an exposed boundary:

```bash
bash scripts/run_security_regressions.sh
```

This checks dependency policy plus adversarial MCP and manifest regressions;
the normal workspace tests cover the broader Rust behavior. The MCP protocol,
capability limits, and audit-receipt boundary are documented in the [MCP
guide](https://kent-tokyo.github.io/renkin/guides/mcp/).

After a fix is available, maintainers will coordinate a disclosure date with
the reporter where practical, credit them with permission, and publish the
affected versions, severity, mitigation, and upgrade guidance.
