#!/usr/bin/env python3

from __future__ import annotations

import os
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
RUN_PAIRED_SCRIPT = REPO_ROOT / "scripts" / "benchmarks" / "run_paired.sh"


class RunPairedScriptTests(unittest.TestCase):
    def test_rounds_alternate_which_binary_runs_first(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            fixtures = temp_root / "fixtures"
            output_dir = temp_root / "out"
            fake_bin = temp_root / "bin"
            hyperfine_log = temp_root / "hyperfine.log"

            fixtures.mkdir()
            fake_bin.mkdir()
            (fixtures / "sample.sh").write_text("#!/bin/sh\necho sample\n")

            # Log the -n names of each hyperfine call, in order, on one line.
            fake_hyperfine = fake_bin / "hyperfine"
            fake_hyperfine.write_text(
                textwrap.dedent(
                    f"""\
                    #!/bin/sh
                    names=
                    export_path=
                    while [ "$#" -gt 0 ]; do
                        case "$1" in
                            -n) names="$names $2"; shift 2 ;;
                            --export-json) export_path=$2; shift 2 ;;
                            *) shift ;;
                        esac
                    done
                    printf '%s ->%s\\n' "$export_path" "$names" >> "{hyperfine_log}"
                    exit 0
                    """
                )
            )
            fake_hyperfine.chmod(0o755)

            shuck_base = fake_bin / "shuck-base"
            shuck_head = fake_bin / "shuck-head"
            for binary in (shuck_base, shuck_head):
                binary.write_text("#!/bin/sh\nexit 0\n")
                binary.chmod(0o755)

            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env['PATH']}",
                    "SHUCK_BENCHMARK_BASE_BIN": str(shuck_base),
                    "SHUCK_BENCHMARK_HEAD_BIN": str(shuck_head),
                    "SHUCK_BENCHMARK_FIXTURES_DIR": str(fixtures),
                    "SHUCK_BENCHMARK_OUTPUT_DIR": str(output_dir),
                    "SHUCK_BENCHMARK_ROUNDS": "3",
                }
            )

            subprocess.run(
                ["sh", str(RUN_PAIRED_SCRIPT)],
                check=True,
                cwd=REPO_ROOT,
                env=env,
                capture_output=True,
            )

            lines = hyperfine_log.read_text().splitlines()

        self.assertEqual(
            lines,
            [
                f"{output_dir}/round-1/bench-sample.json -> base/sample head/sample",
                f"{output_dir}/round-1/bench-all.json -> base/all head/all",
                f"{output_dir}/round-2/bench-sample.json -> head/sample base/sample",
                f"{output_dir}/round-2/bench-all.json -> head/all base/all",
                f"{output_dir}/round-3/bench-sample.json -> base/sample head/sample",
                f"{output_dir}/round-3/bench-all.json -> base/all head/all",
            ],
        )

    def test_missing_binary_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_root = Path(temp_dir)
            env = os.environ.copy()
            env.update(
                {
                    "SHUCK_BENCHMARK_BASE_BIN": str(temp_root / "missing-base"),
                    "SHUCK_BENCHMARK_HEAD_BIN": str(temp_root / "missing-head"),
                    "SHUCK_BENCHMARK_OUTPUT_DIR": str(temp_root / "out"),
                }
            )

            result = subprocess.run(
                ["sh", str(RUN_PAIRED_SCRIPT)],
                cwd=REPO_ROOT,
                env=env,
                capture_output=True,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing-base", result.stderr)


if __name__ == "__main__":
    unittest.main()
