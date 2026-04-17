#!/usr/bin/env python3

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from macro_pr_report import load_macro_result


class MacroPrReportTests(unittest.TestCase):
    def test_selects_shuck_result_by_command_name(self) -> None:
        payload = {
            "results": [
                {
                    "command": "shellcheck/all",
                    "mean": 9.9,
                    "stddev": 0.1,
                    "min": 9.8,
                    "max": 10.0,
                    "exit_codes": [0],
                },
                {
                    "command": "shuck/all",
                    "mean": 1.2,
                    "stddev": 0.05,
                    "min": 1.1,
                    "max": 1.3,
                    "exit_codes": [1],
                },
            ]
        }

        with tempfile.TemporaryDirectory() as temp_dir:
            path = Path(temp_dir) / "bench-all.json"
            path.write_text(json.dumps(payload))

            result = load_macro_result(path)

        self.assertIsNotNone(result)
        assert result is not None
        self.assertEqual(result.case, "all")
        self.assertEqual(result.mean, 1.2)
        self.assertEqual(result.exit_codes, (1,))


if __name__ == "__main__":
    unittest.main()
