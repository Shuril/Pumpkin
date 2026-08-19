import json
import tempfile
import unittest
from pathlib import Path

from parity_runner import canonicalize, load_scenario


class ParityRunnerTests(unittest.TestCase):
    def test_canonicalize_removes_volatile_fields_but_keeps_list_order(self):
        value = {
            "timestamp": 10,
            "uuid": "123e4567-e89b-12d3-a456-426614174000",
            "items": [{"b": 2, "a": 1}, {"a": 3}],
        }
        self.assertEqual(
            canonicalize(value),
            {"items": [{"a": 1, "b": 2}, {"a": 3}], "uuid": "<uuid>"},
        )

    def test_json_and_small_yaml_scenarios(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            json_path = root / "scenario.json"
            json_path.write_text(json.dumps([{"command": "seed"}, {"wait_ticks": 2}]))
            self.assertEqual(load_scenario(json_path)[1]["wait_ticks"], 2)

            yaml_path = root / "scenario.yaml"
            yaml_path.write_text('- command: "seed"\n- wait_ticks: 3\n')
            self.assertEqual(load_scenario(yaml_path), [{"command": "seed"}, {"wait_ticks": 3}])


if __name__ == "__main__":
    unittest.main()
