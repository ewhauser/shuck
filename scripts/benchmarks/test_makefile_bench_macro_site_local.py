#!/usr/bin/env python3

from __future__ import annotations

import subprocess
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]


class MakefileBenchMacroSiteLocalTests(unittest.TestCase):
    def test_export_uses_configured_benchmark_output_directory(self) -> None:
        result = subprocess.run(
            [
                "make",
                "-n",
                "bench-macro-site-local",
                "SHUCK_BENCHMARK_OUTPUT_DIR=/tmp/bench-output",
                "NIX_DEVELOP=",
            ],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        )

        self.assertIn('--bench-dir "/tmp/bench-output"', result.stdout)


if __name__ == "__main__":
    unittest.main()
