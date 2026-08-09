# Test fixtures for `scripts/ord_evidence_audit.py`

`ord_evidence_fixture.pbtxt` is a **hand-authored, synthetic** ORD `Dataset`
message (protobuf text format) written for this test suite. It is not derived
from, or copied out of, any real ORD dataset record — the reaction (an ester
formation from acetic acid and ethanol), conditions, yield, and DOI are all
illustrative values invented to exercise the converter's field mapping.

Because it contains no real experimental data, this file is licensed the same
as the rest of the RENKIN repository (MIT), not CC-BY-SA-4.0 (the license that
applies to real ORD reaction data — see `docs/guides/reaction-evidence.md`).

Field shapes follow the public ORD schema
(https://github.com/Open-Reaction-Database/ord-schema, Apache-2.0), so the
fixture is structurally realistic, but no content was copied from that
project's data or examples.
