#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
from dataclasses import dataclass
from pathlib import Path


COMMENT_MARKER = "<!-- shuck-benchmark-report -->"
REGRESSION_THRESHOLD = 5.0


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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate a Markdown benchmark report from macro CLI benchmark JSON exports."
    )
    parser.add_argument("--baseline-dir", type=Path, required=True)
    parser.add_argument("--head-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--base-sha", required=True)
    parser.add_argument("--head-sha", required=True)
    parser.add_argument("--baseline-source", default="prepared in this workflow")
    parser.add_argument("--baseline-outcome", default="success")
    parser.add_argument("--compare-outcome", default="success")
    parser.add_argument("--run-url", default="")
    return parser.parse_args()


def load_macro_result(path: Path) -> MacroResult | None:
    payload = json.loads(path.read_text())
    results = payload.get("results", [])
    if not results:
        return None

    shuck = results[0]
    case = path.stem.removeprefix("bench-")
    return MacroResult(
        case=case,
        mean=float(shuck["mean"]),
        stddev=float(shuck.get("stddev", 0.0)),
        minimum=float(shuck.get("min", shuck["mean"])),
        maximum=float(shuck.get("max", shuck["mean"])),
        exit_codes=tuple(int(code) for code in shuck.get("exit_codes", [])),
    )


def collect_rows(baseline_dir: Path, head_dir: Path) -> list[BenchmarkRow]:
    if not (baseline_dir.exists() and head_dir.exists()):
        return []

    baseline_paths = {path.name: path for path in baseline_dir.glob("bench-*.json")}
    head_paths = {path.name: path for path in head_dir.glob("bench-*.json")}
    shared_names = sorted(
        baseline_paths.keys() & head_paths.keys(),
        key=lambda name: (name != "bench-all.json", name),
    )

    rows: list[BenchmarkRow] = []
    for name in shared_names:
        baseline = load_macro_result(baseline_paths[name])
        current = load_macro_result(head_paths[name])
        if baseline is None or current is None:
            continue
        rows.append(BenchmarkRow(baseline=baseline, current=current))
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


def render_report(args: argparse.Namespace, rows: list[BenchmarkRow]) -> str:
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
    flagged = [row for row in aggregate if row.delta_pct >= REGRESSION_THRESHOLD]
    failed_rows = [row for row in rows if row.baseline.has_failures or row.current.has_failures]

    lines = [
        COMMENT_MARKER,
        "## Macro CLI Benchmark Deltas",
        "",
        f"Compared `{head_short}` against macro CLI baseline `{base_short}`.",
        "",
        "Positive deltas mean slower `shuck check --no-cache` runs.",
        "",
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
            f"Flagged aggregate regressions over +{REGRESSION_THRESHOLD:.1f}%: {len(flagged)}",
        ]
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
    rows = collect_rows(args.baseline_dir, args.head_dir)
    report = render_report(args, rows)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(report)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
