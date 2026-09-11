import unittest

from scripts.verify_four_tool_images import registry_images


class FourToolImageTests(unittest.TestCase):
    def test_registry_declares_container_images_for_external_arms(self):
        images = registry_images("benchmarks/four_tool/configuration_registry.json")
        self.assertIn("renkin-bench/syntheseus:0.8.0", images)
        self.assertIn("renkin-bench/synplanner:1.6.0", images)


if __name__ == "__main__":
    unittest.main()
