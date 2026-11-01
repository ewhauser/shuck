#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import tempfile
import unittest
from pathlib import Path

from macro_pr_report import (
    COMMENT_MARKER,
    BenchmarkRow,
    MacroResult,
    collect_rows,
    render_report,
)


def macro_result(mean: float, stddev: float, case: str = "all") -> MacroResult:
    return MacroResult(
        case=case,
        mean=mean,
        stddev=stddev,
        minimum=mean - stddev,
        maximum=mean + stddev,
        exit_codes=(0,),
    )


def hyperfine_result(command: str, times: list[float], exit_code: int = 0) -> dict:
    return {
        "command": command,
        "mean": sum(times) / len(times),
        "stddev": 0.0,
        "min": min(times),
        "max": max(times),
        "times": times,
        "exit_codes": [exit_code] * len(times),
    }


def write_round(
    paired_dir: Path, round_number: int, case: str, results: list[dict]
) -> None:
    round_dir = paired_dir / f"round-{round_number}"
    round_dir.mkdir(parents=True, exist_ok=True)
    (round_dir / f"bench-{case}.json").write_text(json.dumps({"results": results}))


def report_args() -> argparse.Namespace:
    return argparse.Namespace(
        base_sha="a" * 40,
        head_sha="b" * 40,
        baseline_source="binary from the latest successful main CI artifact",
        baseline_outcome="success",
        compare_outcome="success",
        run_url="",
    )


class RegressionFlagTests(unittest.TestCase):
    def test_flags_delta_over_threshold_and_noise(self) -> None:
        row = BenchmarkRow(
            baseline=macro_result(1.00, 0.01), current=macro_result(1.10, 0.01)
        )

        self.assertAlmostEqual(row.delta_pct, 10.0)
        self.assertTrue(row.is_regression)

    def test_ignores_delta_under_threshold_even_when_quiet(self) -> None:
        row = BenchmarkRow(
            baseline=macro_result(1.00, 0.001), current=macro_result(1.04, 0.001)
        )

        self.assertFalse(row.is_regression)

    def test_ignores_delta_within_noise(self) -> None:
        # 8% slower, but each side wobbles by ~5%, so 2x the combined noise
        # (~14%) swallows the delta.
        row = BenchmarkRow(
            baseline=macro_result(1.00, 0.05), current=macro_result(1.08, 0.054)
        )

        self.assertGreater(row.delta_pct, 5.0)
        self.assertGreater(2.0 * row.noise_pct, row.delta_pct)
        self.assertFalse(row.is_regression)

    def test_improvements_are_never_flagged(self) -> None:
        row = BenchmarkRow(
            baseline=macro_result(1.00, 0.0), current=macro_result(0.80, 0.0)
        )

        self.assertFalse(row.is_regression)


class CollectRowsTests(unittest.TestCase):
    def test_pools_samples_across_rounds_regardless_of_order(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            paired_dir = Path(temp_dir)
            write_round(
                paired_dir,
                1,
                "all",
                [
                    hyperfine_result("base/all", [1.0, 1.2]),
                    hyperfine_result("head/all", [2.0, 2.2]),
                ],
            )
            # Round two runs head first; the side comes from the command name.
            write_round(
                paired_dir,
                2,
                "all",
                [
                    hyperfine_result("head/all", [2.4, 2.6], exit_code=1),
                    hyperfine_result("base/all", [1.4, 1.6]),
                ],
            )

            rows = collect_rows(paired_dir)

        self.assertEqual(len(rows), 1)
        row = rows[0]
        self.assertEqual(row.case, "all")
        self.assertAlmostEqual(row.baseline.mean, 1.3)
        self.assertAlmostEqual(row.current.mean, 2.3)
        self.assertAlmostEqual(row.baseline.minimum, 1.0)
        self.assertAlmostEqual(row.current.maximum, 2.6)
        self.assertEqual(row.current.exit_codes, (0, 0, 1, 1))
        self.assertTrue(row.current.has_failures)
        self.assertFalse(row.baseline.has_failures)

    def test_aggregate_sorts_first_and_one_sided_cases_are_skipped(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            paired_dir = Path(temp_dir)
            for case in ("zeta", "all", "alpha"):
                write_round(
                    paired_dir,
                    1,
                    case,
                    [
                        hyperfine_result(f"base/{case}", [1.0]),
                        hyperfine_result(f"head/{case}", [1.0]),
                    ],
                )
            write_round(
                paired_dir, 1, "orphan", [hyperfine_result("base/orphan", [1.0])]
            )

            rows = collect_rows(paired_dir)

        self.assertEqual([row.case for row in rows], ["all", "alpha", "zeta"])

    def test_missing_directory_yields_no_rows(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            self.assertEqual(collect_rows(Path(temp_dir) / "missing"), [])


class RenderReportTests(unittest.TestCase):
    def test_report_flags_only_real_aggregate_regressions(self) -> None:
        rows = [
            BenchmarkRow(
                baseline=macro_result(1.00, 0.01),
                current=macro_result(1.10, 0.01),
            ),
            # A noisy fixture regression must not raise the flagged count.
            BenchmarkRow(
                baseline=macro_result(0.010, 0.002, case="nvm"),
                current=macro_result(0.012, 0.002, case="nvm"),
            ),
        ]

        report = render_report(report_args(), rows, rounds=3)

        self.assertTrue(report.startswith(COMMENT_MARKER))
        self.assertIn("| Benchmark | Baseline | Head | Delta |", report)
        self.assertIn("same runner in 3 alternating rounds", report)
        self.assertIn("- Flagged aggregate regressions over +5.0% and 2x noise: 1", report)

    def test_report_does_not_flag_noisy_aggregate(self) -> None:
        rows = [
            BenchmarkRow(
                baseline=macro_result(1.00, 0.05),
                current=macro_result(1.08, 0.054),
            )
        ]

        report = render_report(report_args(), rows, rounds=3)

        self.assertIn("- Flagged aggregate regressions over +5.0% and 2x noise: 0", report)
        self.assertIn("+8.00%", report)


if __name__ == "__main__":
    unittest.main()
