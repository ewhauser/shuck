#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from pathlib import Path


COMMENT_MARKER = "<!-- shuck-benchmark-report -->"
REGRESSION_THRESHOLD = 3.0


@dataclass(frozen=True)
class Estimate:
    point: float
    lower: float
    upper: float


@dataclass(frozen=True)
class BenchmarkRow:
    group: str
    case: str
    baseline: Estimate
    current: Estimate
    change: Estimate

    @property
    def slug(self) -> str:
        return f"{self.group}/{self.case}"

    @property
    def percent_change(self) -> float:
        return self.change.point * 100.0

    @property
    def ci(self) -> str:
        return f"{self.change.lower * 100.0:+.2f}% to {self.change.upper * 100.0:+.2f}%"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate a Markdown benchmark report from Criterion baseline artifacts."
    )
    parser.add_argument("--criterion-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--base-sha", required=True)
    parser.add_argument("--head-sha", required=True)
    parser.add_argument("--baseline-name", default="pr-base")
    parser.add_argument("--baseline-outcome", default="success")
    parser.add_argument("--compare-outcome", default="success")
    parser.add_argument("--run-url", default="")
    return parser.parse_args()


def load_estimate(path: Path) -> Estimate:
    payload = json.loads(path.read_text())
    mean = payload["mean"]
    interval = mean["confidence_interval"]
    return Estimate(
        point=float(mean["point_estimate"]),
        lower=float(interval["lower_bound"]),
        upper=float(interval["upper_bound"]),
    )


def collect_rows(criterion_root: Path, baseline_name: str) -> list[BenchmarkRow]:
    rows: list[BenchmarkRow] = []
    if not criterion_root.exists():
        return rows

    for group_dir in sorted(path for path in criterion_root.iterdir() if path.is_dir()):
        for case_dir in sorted(path for path in group_dir.iterdir() if path.is_dir()):
            baseline_path = case_dir / baseline_name / "estimates.json"
            current_path = case_dir / "new" / "estimates.json"
            change_path = case_dir / "change" / "estimates.json"
            if not (baseline_path.is_file() and current_path.is_file() and change_path.is_file()):
                continue

            rows.append(
                BenchmarkRow(
                    group=group_dir.name,
                    case=case_dir.name,
                    baseline=load_estimate(baseline_path),
                    current=load_estimate(current_path),
                    change=load_estimate(change_path),
                )
            )

    return rows


def format_duration_ns(value: float) -> str:
    units = [
        ("s", 1_000_000_000.0),
        ("ms", 1_000_000.0),
        ("us", 1_000.0),
        ("ns", 1.0),
    ]
    absolute = abs(value)
    for suffix, scale in units:
        if absolute >= scale or suffix == "ns":
            return f"{value / scale:.2f} {suffix}"
    raise AssertionError("unreachable")


def aggregate_rows(rows: list[BenchmarkRow]) -> list[BenchmarkRow]:
    preferred = [row for row in rows if row.case == "all"]
    return preferred if preferred else rows


def render_table(rows: list[BenchmarkRow], include_timings: bool) -> list[str]:
    if not rows:
        return ["No benchmark deltas were produced."]

    header = (
        "| Benchmark | Baseline | Head | Delta | 95% CI |"
        if include_timings
        else "| Benchmark | Delta | 95% CI |"
    )
    separator = (
        "| --- | ---: | ---: | ---: | --- |"
        if include_timings
        else "| --- | ---: | --- |"
    )
    lines = [header, separator]

    for row in rows:
        delta = f"{row.percent_change:+.2f}%"
        if include_timings:
            lines.append(
                "| {slug} | {baseline} | {current} | {delta} | {ci} |".format(
                    slug=row.slug,
                    baseline=format_duration_ns(row.baseline.point),
                    current=format_duration_ns(row.current.point),
                    delta=delta,
                    ci=row.ci,
                )
            )
        else:
            lines.append(f"| {row.slug} | {delta} | {row.ci} |")

    return lines


def render_report(args: argparse.Namespace, rows: list[BenchmarkRow]) -> str:
    base_short = args.base_sha[:7]
    head_short = args.head_sha[:7]

    aggregate = aggregate_rows(rows)
    regressions = sorted(
        [row for row in rows if row.percent_change > 0.0],
        key=lambda row: row.percent_change,
        reverse=True,
    )[:5]
    improvements = sorted(
        [row for row in rows if row.percent_change < 0.0],
        key=lambda row: row.percent_change,
    )[:5]
    flagged = [row for row in aggregate if row.percent_change >= REGRESSION_THRESHOLD]

    lines = [
        COMMENT_MARKER,
        "## Benchmark Deltas",
        "",
        f"Compared `{head_short}` against Criterion baseline `{base_short}`.",
        "",
        "Positive deltas mean slower benchmark runs.",
        "",
        f"- Baseline run: `{args.baseline_outcome}`",
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

    lines.extend(["", "### Aggregate Cases", ""])
    lines.extend(render_table(aggregate, include_timings=True))

    lines.extend(
        [
            "",
            f"Flagged aggregate regressions over +{REGRESSION_THRESHOLD:.1f}%: {len(flagged)}",
        ]
    )

    if regressions or improvements:
        lines.extend(["", "<details>", "<summary>Largest per-case changes</summary>", ""])
        if regressions:
            lines.extend(["#### Regressions", ""])
            lines.extend(render_table(regressions, include_timings=False))
            lines.append("")
        if improvements:
            lines.extend(["#### Improvements", ""])
            lines.extend(render_table(improvements, include_timings=False))
            lines.append("")
        lines.append("</details>")

    if not rows:
        lines.extend(
            [
                "",
                "No Criterion comparison data was found under the shared benchmark target directory.",
            ]
        )

    return "\n".join(lines).rstrip() + "\n"


def main() -> int:
    args = parse_args()
    rows = collect_rows(args.criterion_root, args.baseline_name)
    report = render_report(args, rows)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(report)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
