#!/usr/bin/env python3

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from scripts.benchmarks.export_website_data import load_cases


class ExportWebsiteDataTests(unittest.TestCase):
    def test_load_cases_ignores_stale_fixture_exports(self) -> None:
        fixtures = [
            {
                "slug": "fixture",
                "name": "fixture",
                "fileName": "fixture.sh",
                "path": "files/fixture.sh",
                "bytes": 12,
                "lines": 2,
                "upstreamRepo": "example/repo",
                "upstreamPath": "fixture.sh",
                "sourceUrl": "https://example.invalid/fixture.sh",
                "license": "MIT",
                "commit": "abc1234",
                "commitShort": "abc1234",
            }
        ]
        fixtures_by_slug = {fixture["slug"]: fixture for fixture in fixtures}

        with tempfile.TemporaryDirectory() as temp_dir:
            bench_dir = Path(temp_dir)
            aggregate_payload = {
                "results": [
                    {
                        "command": "shuck/all",
                        "mean": 1.0,
                        "stddev": 0.1,
                        "median": 1.0,
                        "min": 0.9,
                        "max": 1.1,
                        "exit_codes": [0],
                    }
                ]
            }
            fixture_payload = {
                "results": [
                    {
                        "command": "shuck/fixture",
                        "mean": 0.5,
                        "stddev": 0.05,
                        "median": 0.5,
                        "min": 0.45,
                        "max": 0.55,
                        "exit_codes": [0],
                    }
                ]
            }
            stale_payload = {
                "results": [
                    {
                        "command": "shuck/stale-fixture",
                        "mean": 9.9,
                        "stddev": 0.1,
                        "median": 9.9,
                        "min": 9.8,
                        "max": 10.0,
                        "exit_codes": [0],
                    }
                ]
            }

            (bench_dir / "bench-all.json").write_text(json.dumps(aggregate_payload))
            (bench_dir / "bench-fixture.json").write_text(json.dumps(fixture_payload))
            (bench_dir / "bench-stale-fixture.json").write_text(json.dumps(stale_payload))

            cases = load_cases(bench_dir, fixtures, fixtures_by_slug)

        self.assertEqual([case["slug"] for case in cases], ["all", "fixture"])
        self.assertEqual(cases[1]["bytes"], 12)
        self.assertEqual(cases[1]["lines"], 2)


if __name__ == "__main__":
    unittest.main()
