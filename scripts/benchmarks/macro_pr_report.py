#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import math
import statistics
from dataclasses import dataclass
from pathlib import Path


COMMENT_MARKER = "<!-- shuck-benchmark-report -->"
REGRESSION_THRESHOLD = 5.0
# A delta only counts as a regression when it also clears this many multiples
# of the combined relative stddev of the two sides.
NOISE_MULTIPLIER = 2.0


@dataclass(frozen=True)
class MacroResult:
    case: str
    mean: float
    stddev: float
    minimum: float
    maximum: float
    exit_codes: tuple[int, ...]

    @property
    def has_failures(self) -> bool:
        return any(code != 0 for code in self.exit_codes)

    @property
    def relative_stddev_pct(self) -> float:
        if self.mean == 0.0:
            return 0.0
        return (self.stddev / self.mean) * 100.0


@dataclass(frozen=True)
class BenchmarkRow:
    baseline: MacroResult
    current: MacroResult

    @property
    def case(self) -> str:
        return self.current.case

    @property
    def delta_pct(self) -> float:
        if self.baseline.mean == 0.0:
            return 0.0
        return ((self.current.mean - self.baseline.mean) / self.baseline.mean) * 100.0

    @property
    def noise_pct(self) -> float:
        """Combined relative stddev of both sides, in percent."""
        return math.hypot(
            self.baseline.relative_stddev_pct, self.current.relative_stddev_pct
        )

    @property
    def is_regression(self) -> bool:
        return (
            self.delta_pct >= REGRESSION_THRESHOLD
            and self.delta_pct > NOISE_MULTIPLIER * self.noise_pct
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate a Markdown benchmark report from macro CLI benchmark JSON exports."
    )
    parser.add_argument(
        "--paired-dir",
        type=Path,
        required=True,
        help="Output directory of run_paired.sh (round-*/bench-*.json).",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--base-sha", required=True)
    parser.add_argument("--head-sha", required=True)
    parser.add_argument("--baseline-source", default="prepared in this workflow")
    parser.add_argument("--baseline-outcome", default="success")
    parser.add_argument("--compare-outcome", default="success")
    parser.add_argument("--run-url", default="")
    return parser.parse_args()


def summarize_samples(
    case: str, times: list[float], exit_codes: list[int]
) -> MacroResult:
    return MacroResult(
        case=case,
        mean=statistics.fmean(times),
        stddev=statistics.stdev(times) if len(times) > 1 else 0.0,
        minimum=min(times),
        maximum=max(times),
        exit_codes=tuple(exit_codes),
    )


def command_samples(result: dict) -> list[float]:
    times = result.get("times")
    if times:
        return [float(sample) for sample in times]
    return [float(result["mean"])]


def load_paired_round(path: Path) -> dict[str, dict]:
    """Return the hyperfine results in one round file keyed by side (base/head)."""
    payload = json.loads(path.read_text())
    sides: dict[str, dict] = {}
    for result in payload.get("results", []):
        side, _, _ = str(result.get("command", "")).partition("/")
        if side in ("base", "head"):
            sides[side] = result
    return sides


def round_dirs(paired_dir: Path) -> list[Path]:
    if not paired_dir.exists():
        return []
    return sorted(path for path in paired_dir.glob("round-*") if path.is_dir())


def collect_rows(paired_dir: Path) -> list[BenchmarkRow]:
    """Pool every round's samples per case and side into one row per case."""
    samples: dict[str, dict[str, list[float]]] = {}
    exit_codes: dict[str, dict[str, list[int]]] = {}
    for round_dir in round_dirs(paired_dir):
        for path in round_dir.glob("bench-*.json"):
            case = path.stem.removeprefix("bench-")
            for side, result in load_paired_round(path).items():
                samples.setdefault(case, {}).setdefault(side, []).extend(
                    command_samples(result)
                )
                exit_codes.setdefault(case, {}).setdefault(side, []).extend(
                    int(code) for code in result.get("exit_codes", [])
                )

    cases = sorted(samples, key=lambda case: (case != "all", case))
    rows: list[BenchmarkRow] = []
    for case in cases:
        sides = samples[case]
        if not (sides.get("base") and sides.get("head")):
            continue
        rows.append(
            BenchmarkRow(
                baseline=summarize_samples(
                    case, sides["base"], exit_codes[case].get("base", [])
                ),
                current=summarize_samples(
                    case, sides["head"], exit_codes[case].get("head", [])
                ),
            )
        )
    return rows


def format_duration(seconds: float) -> str:
    absolute = abs(seconds)
    if absolute >= 1.0:
        return f"{seconds:.2f} s"
    if absolute >= 0.001:
        return f"{seconds * 1_000.0:.2f} ms"
    return f"{seconds * 1_000_000.0:.2f} us"


def format_mean_stddev(result: MacroResult) -> str:
    return f"{format_duration(result.mean)} +/- {format_duration(result.stddev)}"


def format_exit_codes(result: MacroResult) -> str:
    if not result.exit_codes:
        return "none recorded"
    return ",".join(str(code) for code in result.exit_codes)


def render_table(rows: list[BenchmarkRow]) -> list[str]:
    if not rows:
        return ["No macro CLI benchmark deltas were produced."]

    lines = [
        "| Benchmark | Baseline | Head | Delta |",
        "| --- | ---: | ---: | ---: |",
    ]
    for row in rows:
        lines.append(
            "| {case} | {baseline} | {current} | {delta:+.2f}% |".format(
                case=row.case,
                baseline=format_mean_stddev(row.baseline),
                current=format_mean_stddev(row.current),
                delta=row.delta_pct,
            )
        )
    return lines


def render_report(
    args: argparse.Namespace, rows: list[BenchmarkRow], rounds: int
) -> str:
    base_short = args.base_sha[:7]
    head_short = args.head_sha[:7]

    aggregate = [row for row in rows if row.case == "all"]
    if not aggregate and rows:
        aggregate = [rows[0]]

    regressions = sorted(
        [row for row in rows if row.delta_pct > 0.0],
        key=lambda row: row.delta_pct,
        reverse=True,
    )[:5]
    improvements = sorted(
        [row for row in rows if row.delta_pct < 0.0],
        key=lambda row: row.delta_pct,
    )[:5]
    flagged = [row for row in aggregate if row.is_regression]
    failed_rows = [row for row in rows if row.baseline.has_failures or row.current.has_failures]

    lines = [
        COMMENT_MARKER,
        "## Macro CLI Benchmark Deltas",
        "",
        f"Compared `{head_short}` against macro CLI baseline `{base_short}`.",
        "",
        "Positive deltas mean slower `shuck check --no-cache` runs.",
        "",
        "- Both binaries were timed on the same runner in "
        f"{rounds} alternating round{'s' if rounds != 1 else ''}, "
        "so differences between CI machines do not show up as deltas.",
        f"- Baseline source: {args.baseline_source}",
        f"- Baseline preparation: `{args.baseline_outcome}`",
        f"- Comparison run: `{args.compare_outcome}`",
    ]

    if args.run_url:
        lines.append(f"- Workflow run: [details]({args.run_url})")

    if args.baseline_outcome != "success" or args.compare_outcome != "success":
        lines.extend(
            [
                "",
                "Benchmark execution did not complete cleanly, so the tables below may be partial.",
            ]
        )

    lines.extend(["", "### Aggregate", ""])
    lines.extend(render_table(aggregate))
    lines.extend(
        [
            "",
            f"- Flagged aggregate regressions over +{REGRESSION_THRESHOLD:.1f}% "
            f"and {NOISE_MULTIPLIER:g}x noise: {len(flagged)}",
        ]
    )
    for row in aggregate:
        lines.append(
            f"- Noise for `{row.case}`: +/-{row.noise_pct:.2f}% "
            "(combined relative stddev of both sides)"
        )

    if len(rows) > len(aggregate):
        lines.extend(["", "<details>", "<summary>Per-fixture macro deltas</summary>", ""])
        lines.extend(render_table(rows))
        lines.extend(["", "</details>"])

    if regressions or improvements:
        lines.extend(["", "<details>", "<summary>Largest changes</summary>", ""])
        if regressions:
            lines.extend(["#### Regressions", ""])
            lines.extend(render_table(regressions))
            lines.append("")
        if improvements:
            lines.extend(["#### Improvements", ""])
            lines.extend(render_table(improvements))
            lines.append("")
        lines.append("</details>")

    if failed_rows:
        lines.extend(["", "<details>", "<summary>Non-zero benchmark exit codes</summary>", ""])
        for row in failed_rows:
            lines.append(
                "- `{case}` baseline exit codes: `{baseline_codes}`; head exit codes: `{head_codes}`".format(
                    case=row.case,
                    baseline_codes=format_exit_codes(row.baseline),
                    head_codes=format_exit_codes(row.current),
                )
            )
        lines.extend(["", "</details>"])

    if not rows:
        lines.extend(
            [
                "",
                "No macro CLI comparison data was found under the benchmark export directories.",
            ]
        )

    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    args = parse_args()
    rows = collect_rows(args.paired_dir)
    report = render_report(args, rows, len(round_dirs(args.paired_dir)))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(report)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
