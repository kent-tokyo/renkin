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

## Disclosure

Please allow reasonable time for a fix before public disclosure.
Reporters will be credited in the advisory unless they prefer anonymity.
