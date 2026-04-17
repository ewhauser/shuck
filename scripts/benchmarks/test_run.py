#!/usr/bin/env python3

from __future__ import annotations

import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
RUN_SCRIPT = REPO_ROOT / "scripts" / "benchmarks" / "run.sh"


class RunBenchmarkScriptTests(unittest.TestCase):
    def test_fixture_override_controls_bench_corpus(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            repo_root = temp_root / "repo"
            default_fixtures = repo_root / "crates" / "shuck-benchmark" / "resources" / "files"
            override_fixtures = temp_root / "override-fixtures"
            output_dir = temp_root / "out"
            fake_bin = temp_root / "bin"
            hyperfine_log = temp_root / "hyperfine.log"

            default_fixtures.mkdir(parents=True)
            override_fixtures.mkdir(parents=True)
            output_dir.mkdir()
            fake_bin.mkdir()

            (default_fixtures / "repo-only.sh").write_text("#!/bin/sh\necho repo\n")
            (override_fixtures / "override-only.sh").write_text("#!/bin/sh\necho override\n")

            fake_hyperfine = fake_bin / "hyperfine"
            fake_hyperfine.write_text(
                textwrap.dedent(
                    f"""\
                    #!/bin/sh
                    printf '%s\\n' "$*" >> "{hyperfine_log}"
                    exit 0
                    """
                )
            )
            fake_hyperfine.chmod(0o755)

            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env['PATH']}",
                    "SHUCK_BENCHMARK_MODE": "shuck-only",
                    "SHUCK_BENCHMARK_OUTPUT_DIR": str(output_dir),
                    "SHUCK_BENCHMARK_REPO_ROOT": str(repo_root),
                    "SHUCK_BENCHMARK_FIXTURES_DIR": str(override_fixtures),
                }
            )

            subprocess.run(
                ["sh", str(RUN_SCRIPT)],
                check=True,
                cwd=REPO_ROOT,
                env=env,
            )

            log = hyperfine_log.read_text()
            self.assertIn("override-only", log)
            self.assertNotIn("repo-only", log)


if __name__ == "__main__":
    unittest.main()
