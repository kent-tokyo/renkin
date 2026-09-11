import json
import unittest
from pathlib import Path

from scripts.validate_four_tool_registry import validate_registry


def load_registry() -> dict:
    path = Path(__file__).parents[2] / "benchmarks" / "four_tool" / "configuration_registry.json"
    return json.loads(path.read_text(encoding="utf-8"))


class ValidateFourToolRegistryTests(unittest.TestCase):
    def test_checked_in_registry_is_structurally_valid(self):
        self.assertEqual(validate_registry(load_registry(), Path.cwd()), [])

    def test_rejects_missing_tool(self):
        payload = load_registry()
        payload["arms"] = payload["arms"][:-1]
        problems = validate_registry(payload, Path.cwd())
        self.assertTrue(any("arm tool set" in problem for problem in problems))

    def test_artifact_check_is_explicit(self):
        payload = load_registry()
        problems = validate_registry(
            payload, Path("/definitely-missing-four-tool-artifacts"), check_artifacts=True
        )
        self.assertTrue(any("missing artifact" in problem for problem in problems))

    def test_not_measured_requires_reason(self):
        payload = load_registry()
        payload["arms"][0]["status"] = "not_measured"
        problems = validate_registry(payload, Path.cwd())
        self.assertTrue(any("requires reason" in problem for problem in problems))

    def test_formal_gate_rejects_candidate_or_pending_runtime(self):
        payload = load_registry()
        problems = validate_registry(payload, Path.cwd(), formal=True)
        self.assertTrue(any("formal" in problem or "verified" in problem for problem in problems))

    def test_formal_gate_requires_immutable_docker_identity(self):
        payload = load_registry()
        for arm in payload["arms"]:
            arm["status"] = "verified"
            arm["runtime"]["resource_enforcement"] = "docker_run_8cpu_6g_network_none"
        problems = validate_registry(payload, Path.cwd(), formal=True, image_identities={"images": []})
        self.assertTrue(any("image identity is missing" in problem for problem in problems))

    def test_formal_gate_accepts_declared_docker_identity(self):
        payload = load_registry()
        identities = []
        for arm in payload["arms"]:
            arm["status"] = "verified"
            arm["runtime"]["resource_enforcement"] = "docker_run_8cpu_6g_network_none"
            if "image" in arm["runtime"]:
                identities.append({"image": arm["runtime"]["image"], "id": "sha256:" + "a" * 64})
        problems = validate_registry(
            payload, Path.cwd(), formal=True, image_identities={"images": identities}
        )
        self.assertEqual(problems, [])


if __name__ == "__main__":
    unittest.main()
