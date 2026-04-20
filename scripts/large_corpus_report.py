#!/usr/bin/env python3

"""Render a large-corpus compatibility log into a standalone HTML report."""

from __future__ import annotations

import argparse
import html
import re
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Iterable


DIAGNOSTIC_LINE_RE = re.compile(
    r"^  (?P<side>shellcheck-only|shuck-only) "
    r"(?P<rule>[^/\s]+)/(?P<shellcheck>SC\d+) "
    r"(?P<range>\S+) "
    r"(?P<message>.*)$"
)
FIXTURE_LINE_RE = re.compile(r"^/.*$")
MAIN_FAILURE_RE = re.compile(
    r"large corpus compatibility had (?P<blocking>\d+) blocking issue\(s\) "
    r"across (?P<fixtures>\d+) fixture\(s\)"
    r"(?: \((?P<skipped>\d+) skipped unsupported shells\))?"
)
MAIN_SUMMARY_RE = re.compile(
    r"large corpus compatibility summary: "
    r"blocking=(?P<blocking>\d+) "
    r"warnings=(?P<warnings>\d+) "
    r"fixtures=(?P<fixtures>\d+) "
    r"unsupported_shells=(?P<skipped>\d+) "
    r"implementation_diffs=(?P<implementation>\d+) "
    r"mapping_issues=(?P<mapping>\d+) "
    r"reviewed_divergences=(?P<reviewed>\d+) "
    r"harness_warnings=(?P<harness_warnings>\d+) "
    r"harness_failures=(?P<harness_failures>\d+)"
)
MAIN_TIMEOUT_NOTE_RE = re.compile(r"large corpus compatibility note: (?P<note>.+)")
ZSH_FAILURE_RE = re.compile(
    r"large corpus zsh parse had (?P<blocking>\d+) blocking issue\(s\) "
    r"across (?P<fixtures>\d+) fixture\(s\)"
)
PROGRESS_RE = re.compile(r"large corpus: processed (?P<done>\d+)/(?P<total>\d+) fixtures")
WORKER_PANIC_RE = re.compile(
    r"thread '<unnamed>' .*? panicked at (?P<location>[^\n]+):\n(?P<message>[^\n]+)",
    re.DOTALL,
)
KNOWN_LARGE_CORPUS_RULE_ALLOWLIST_REASON = "known large-corpus rule allowlist"


@dataclass
class RuleSummary:
    rule_code: str
    shellcheck_code: str
    description: str
    mismatches: int = 0
    fixtures: set[str] = field(default_factory=set)
    grouped_reasons: Counter[str] = field(default_factory=Counter)

    def top_reason_groups(self, limit: int = 3) -> list[tuple[str, int]]:
        return self.grouped_reasons.most_common(limit)


@dataclass
class BlockerEntry:
    bucket: str
    fixture_path: str
    record_count: int
    spans: tuple[str, ...]
    codes: tuple[str, ...]
    reasons: tuple[str, ...]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", required=True, help="Path to the large-corpus output log")
    parser.add_argument("--output", required=True, help="Path to write the HTML report")
    parser.add_argument(
        "--repo-root",
        default=str(Path(__file__).resolve().parents[1]),
        help="Repository root used to resolve docs/rules metadata",
    )
    return parser.parse_args()


# Ordered section titles as emitted by the Rust test harness.  Any section
# may be absent when it contains zero items.
_SECTION_ORDER = [
    "Implementation Diffs",
    "Mapping Issues",
    "Reviewed Divergence",
    "Harness Warnings",
    "Harness Failures",
]
_OMITTED_NOTE_MARKER = (
    "\n\nNonblocking issue buckets were omitted from the failing log output."
)


def extract_sections(text: str) -> dict[str, str]:
    """Find all present sections and return a {title: body} mapping.

    The Rust harness only emits sections that have content, so we locate each
    section header that actually appears and slice between consecutive headers.
    """
    # Build list of (position, title) for headers that exist.
    found: list[tuple[int, str]] = []
    for title in _SECTION_ORDER:
        pos = text.find(f"{title}:\n")
        if pos != -1:
            found.append((pos, title))
    found.sort()

    result: dict[str, str] = {}
    for idx, (pos, title) in enumerate(found):
        body_start = pos + len(title) + 2  # skip "Title:\n"
        if idx + 1 < len(found):
            body_end = text.rfind("\n", body_start, found[idx + 1][0])
            if body_end == -1:
                body_end = found[idx + 1][0]
        else:
            # Last section — end at the next blank-line-separated block or EOF.
            end_candidates = [len(text)]
            if (omitted_note_match := text.find(_OMITTED_NOTE_MARKER, body_start)) != -1:
                end_candidates.append(omitted_note_match)
            if (end_match := text.find("\n\ntest ", body_start)) != -1:
                end_candidates.append(end_match)
            body_end = min(end_candidates)
        result[title] = text[body_start:body_end].rstrip("\n")
    return result


def require_main_harness_section(text: str, start_at: int = 0) -> str | None:
    start = text.find("Harness Failures:\n", start_at)
    if start == -1:
        return None
    start += len("Harness Failures:\n")
    end_marker = "\ntest large_corpus_conforms_with_shellcheck ... FAILED"
    end = text.find(end_marker, start)
    if end == -1:
        return None
    return text[start:end]


def find_first_section_start(text: str, start_at: int = 0) -> int:
    positions = [
        pos
        for title in _SECTION_ORDER
        if (pos := text.find(f"{title}:\n", start_at)) != -1
    ]
    return min(positions, default=-1)


def extract_main_report_body(text: str) -> str | None:
    anchor_match = MAIN_SUMMARY_RE.search(text) or MAIN_FAILURE_RE.search(text)
    if not anchor_match:
        return None

    end = text.find("\ntest large_corpus_conforms_with_shellcheck ...", anchor_match.end())
    if end == -1:
        end = len(text)

    start = find_first_section_start(text[:end], anchor_match.end())
    if start == -1:
        return None

    return text[start:end].rstrip("\n")


def optional_zsh_harness_section(text: str) -> str | None:
    zsh_match = ZSH_FAILURE_RE.search(text)
    if not zsh_match:
        return None
    start = text.find("Harness Failures:\n", zsh_match.start())
    if start == -1:
        return None
    start += len("Harness Failures:\n")
    end_marker = "\ntest large_corpus_zsh_fixtures_parse ... FAILED"
    end = text.find(end_marker, start)
    if end == -1:
        return None
    return text[start:end]


def parse_rule_metadata(repo_root: Path, rule_code: str) -> tuple[str, str]:
    rule_file = repo_root / "docs" / "rules" / f"{rule_code}.yaml"
    if not rule_file.is_file():
        return "", ""

    description = ""
    shellcheck_code = ""
    for line in rule_file.read_text(encoding="utf-8").splitlines():
        if line.startswith("description: "):
            description = line.split(": ", 1)[1].strip().strip('"')
        elif line.startswith("shellcheck_code: "):
            shellcheck_code = line.split(": ", 1)[1].strip()
        if description and shellcheck_code:
            break
    return description, shellcheck_code


def parse_rule_summaries(section: str | None, repo_root: Path) -> list[RuleSummary]:
    if not section:
        return []
    summaries: dict[str, RuleSummary] = {}
    current_fixture: str | None = None

    for line in section.splitlines():
        if FIXTURE_LINE_RE.match(line):
            current_fixture = line
            continue

        match = DIAGNOSTIC_LINE_RE.match(line)
        if not match:
            continue

        rule_code = match.group("rule")
        summary = summaries.get(rule_code)
        if summary is None:
            description, rule_shellcheck_code = parse_rule_metadata(repo_root, rule_code)
            summary = summaries[rule_code] = RuleSummary(
                rule_code=rule_code,
                shellcheck_code=rule_shellcheck_code or match.group("shellcheck"),
                description=description,
            )

        summary.mismatches += 1
        if current_fixture is not None:
            summary.fixtures.add(current_fixture)
        summary.grouped_reasons[match.group("side")] += 1

    return sorted(summaries.values(), key=lambda summary: summary.mismatches, reverse=True)


def combine_sections(*sections: str | None) -> str | None:
    present = [section for section in sections if section]
    if not present:
        return None
    return "\n\n".join(present)


def filter_reviewed_divergence_section_for_known_failures(section: str | None) -> str | None:
    if not section:
        return None

    kept_entries: list[str] = []
    for fixture_path, lines in parse_fixture_entries(section):
        kept_lines: list[str] = []
        for line in lines:
            stripped = line.strip()
            if not stripped:
                continue
            if f" reason={KNOWN_LARGE_CORPUS_RULE_ALLOWLIST_REASON}" not in stripped:
                continue
            kept_lines.append(line)

        if kept_lines:
            kept_entries.append("\n".join([fixture_path, *kept_lines]))

    if not kept_entries:
        return None
    return "\n\n".join(kept_entries)


def ordered_unique(values: Iterable[str]) -> tuple[str, ...]:
    seen: set[str] = set()
    ordered: list[str] = []
    for value in values:
        if value in seen:
            continue
        seen.add(value)
        ordered.append(value)
    return tuple(ordered)


def parse_fixture_entries(section: str | None) -> list[tuple[str, list[str]]]:
    if not section:
        return []

    entries: list[tuple[str, list[str]]] = []
    current_fixture: str | None = None
    current_lines: list[str] = []

    for line in section.splitlines():
        if FIXTURE_LINE_RE.match(line):
            if current_fixture is not None:
                entries.append((current_fixture, current_lines))
            current_fixture = line
            current_lines = []
            continue

        if current_fixture is not None:
            current_lines.append(line)

    if current_fixture is not None:
        entries.append((current_fixture, current_lines))

    return entries


def default_blocker_reason(bucket: str) -> str:
    if bucket == "Implementation Diff":
        return "Direct implementation mismatch with no comparison-target override."
    if bucket == "Known Failure":
        return "This mismatch is covered by the known large-corpus rule allowlist."
    if bucket == "Harness Warning":
        return "Harness execution hit a non-blocking timeout or warning for this fixture."
    if bucket == "Harness Failure":
        return "Harness execution failed while evaluating this fixture."
    return "Comparison target or rule mapping could not be resolved cleanly."


def parse_blocker_entries(bucket: str, section: str | None) -> list[BlockerEntry]:
    entries: list[BlockerEntry] = []

    for fixture_path, lines in parse_fixture_entries(section):
        detail_lines = [line.strip() for line in lines if line.strip()]
        if not detail_lines:
            continue

        spans: list[str] = []
        codes: list[str] = []
        reasons: list[str] = []

        for line in lines:
            stripped = line.strip()
            if not stripped:
                continue

            match = DIAGNOSTIC_LINE_RE.match(line)
            if match:
                spans.append(match.group("range"))
                codes.append(f"{match.group('rule')}/{match.group('shellcheck')}")
                if " reason=" in stripped:
                    reasons.append(stripped.split(" reason=", 1)[1].strip())
                continue

            reasons.append(stripped)

        if not reasons:
            reasons.append(default_blocker_reason(bucket))

        entries.append(
            BlockerEntry(
                bucket=bucket,
                fixture_path=fixture_path,
                record_count=len(detail_lines),
                spans=ordered_unique(spans),
                codes=ordered_unique(codes) or (bucket,),
                reasons=ordered_unique(reasons),
            )
        )

    return entries


def count_diagnostic_records(section: str | None) -> int:
    if not section:
        return 0
    return sum(1 for line in section.splitlines() if DIAGNOSTIC_LINE_RE.match(line))


def count_fixture_entries(section: str | None) -> int:
    if not section:
        return 0
    return sum(1 for line in section.splitlines() if FIXTURE_LINE_RE.match(line))


def format_number(value: int) -> str:
    return f"{value:,}"


def side_phrase(side: str) -> str:
    return "SC-only" if side == "shellcheck-only" else "Shuck-only"


def side_class(side: str) -> str:
    return "sc-only" if side == "shellcheck-only" else "shuck-only"


def rendered_reason_items(summary: RuleSummary) -> str:
    items: list[str] = []
    top_groups = summary.top_reason_groups()
    top_count = sum(count for _, count in top_groups)
    other_count = summary.mismatches - top_count

    for side, count in top_groups:
        items.append(
            "<li>"
            f'<span class="count-tag">{format_number(count)}</span>'
            f'<span class="{side_class(side)}">{html.escape(side_phrase(side))}</span>'
            "</li>"
        )

    if other_count:
        suffix = "bucket" if other_count == 1 else "buckets"
        items.append(
            "<li>"
            f'<span class="count-tag">{format_number(other_count)}</span>'
            f"other smaller {suffix}"
            "</li>"
        )

    return "\n".join(items)


def main_fixture_total(text: str, fallback_total: int, unsupported_shells: int) -> int:
    totals = [int(match.group("total")) for match in PROGRESS_RE.finditer(text)]
    if totals:
        return max(totals)
    return max(fallback_total - unsupported_shells, 0)


def worker_panic(text: str) -> tuple[str, str] | None:
    match = WORKER_PANIC_RE.search(text)
    if not match:
        return None
    return match.group("location"), match.group("message")


def top_rule_share(rule_summaries: Iterable[RuleSummary], top_n: int = 5) -> float:
    rules = list(rule_summaries)
    total = sum(summary.mismatches for summary in rules)
    if total == 0:
        return 0.0
    top_total = sum(summary.mismatches for summary in rules[:top_n])
    return (top_total / total) * 100.0


def render_detail_list(items: tuple[str, ...], class_name: str) -> str:
    return "\n".join(
        f'<li class="{class_name}">{html.escape(item)}</li>' for item in items
    )


def render_html(
    *,
    log_path: Path,
    output_path: Path,
    generated_at: datetime,
    main_blocking: int,
    main_fixture_entries: int,
    unsupported_shells: int,
    main_processed_fixtures: int,
    rule_records: int,
    shellcheck_only: int,
    shuck_only: int,
    mapping_issues: int,
    reviewed_divergences: int,
    main_harness_warnings: int,
    main_harness_failures: int,
    main_timeout_note: str | None,
    zsh_blocking: int,
    zsh_fixture_entries: int,
    zsh_harness_failures: int,
    top_five_share: float,
    worker_panic_info: tuple[str, str] | None,
    blocker_entries: list[BlockerEntry],
    rule_summaries: list[RuleSummary],
) -> str:
    panic_text = ""
    if worker_panic_info is not None:
        location, message = worker_panic_info
        panic_text = (
            "one worker-thread panic at "
            f"<code>{html.escape(location)}</code> with message "
            f"<code>{html.escape(message)}</code>"
        )
    else:
        panic_text = "no worker-thread panic was detected in the parsed log"
    timeout_note_html = (
        f'\n      <p class="note">{html.escape(main_timeout_note)}</p>'
        if main_timeout_note
        else ""
    )

    rule_rows = "\n".join(
        """
            <tr>
              <td>
                <div class="rule-code"><span class="badge">{rule_code}</span><span>{shellcheck_code}</span></div>
                <p class="rule-desc">{description}</p>
              </td>
              <td><p class="metric">{mismatches}</p><p class="metric-label">rule-coded records</p></td>
              <td><p class="metric">{fixtures}</p><p class="metric-label">{fixture_label}</p></td>
              <td>
                <ul class="reason-list">
                  {reason_items}
                </ul>
              </td>
            </tr>
        """.format(
            rule_code=html.escape(summary.rule_code),
            shellcheck_code=html.escape(summary.shellcheck_code),
            description=html.escape(summary.description or "No rule description found."),
            mismatches=format_number(summary.mismatches),
            fixtures=format_number(len(summary.fixtures)),
            fixture_label="fixture" if len(summary.fixtures) == 1 else "fixtures",
            reason_items=rendered_reason_items(summary),
        ).strip()
        for summary in rule_summaries
    )

    blocker_rows = "\n".join(
        """
            <tr>
              <td><span class="bucket-chip">{bucket}</span></td>
              <td>
                <div class="fixture-name">{fixture_name}</div>
                <div class="fixture-path"><code>{fixture_path}</code></div>
              </td>
              <td>
                <p class="metric">{record_count}</p>
                <p class="metric-label">{record_label}</p>
                {span_list}
              </td>
              <td>
                <div class="chip-list">
                  {code_chips}
                </div>
              </td>
              <td>
                <ul class="detail-list">
                  {reason_items}
                </ul>
              </td>
            </tr>
        """.format(
            bucket=html.escape(entry.bucket),
            fixture_name=html.escape(Path(entry.fixture_path).name),
            fixture_path=html.escape(entry.fixture_path),
            record_count=format_number(entry.record_count),
            record_label="record" if entry.record_count == 1 else "records",
            span_list=(
                '<ul class="detail-list detail-list-tight">'
                f"{render_detail_list(entry.spans, 'detail-item-muted')}"
                "</ul>"
                if entry.spans
                else ""
            ),
            code_chips="\n".join(
                f'<span class="code-chip">{html.escape(code)}</span>'
                for code in entry.codes
            ),
            reason_items=render_detail_list(entry.reasons, "detail-item"),
        ).strip()
        for entry in blocker_entries
    )
    if not blocker_rows:
        blocker_rows = """
            <tr>
              <td colspan="5">
                <p class="metric-label">No issue entries were parsed from the main run.</p>
              </td>
            </tr>
        """.strip()

    generated_label = generated_at.strftime("%B %d, %Y %H:%M %Z").replace(" 0", " ")

    return f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Shuck Large Corpus Report</title>
  <style>
    :root {{
      --bg: #f6f0e2;
      --bg-soft: #fbf7ef;
      --panel: rgba(255, 250, 240, 0.92);
      --panel-strong: #fffaf0;
      --text: #2a2015;
      --muted: #675847;
      --line: rgba(92, 71, 47, 0.16);
      --accent: #115e59;
      --accent-soft: rgba(17, 94, 89, 0.12);
      --warn: #9a3412;
      --shadow: 0 18px 46px rgba(58, 40, 18, 0.12);
      --radius: 18px;
    }}

    * {{
      box-sizing: border-box;
    }}

    body {{
      margin: 0;
      min-height: 100vh;
      font-family: "Avenir Next", "Trebuchet MS", sans-serif;
      color: var(--text);
      background:
        radial-gradient(circle at top left, rgba(17, 94, 89, 0.18), transparent 28%),
        radial-gradient(circle at top right, rgba(154, 52, 18, 0.18), transparent 28%),
        linear-gradient(180deg, #f9f3e7 0%, #f1ead9 100%);
    }}

    .page {{
      width: min(1320px, calc(100vw - 32px));
      margin: 24px auto 40px;
    }}

    .hero {{
      position: relative;
      overflow: hidden;
      padding: 28px 30px;
      border: 1px solid var(--line);
      border-radius: 28px;
      background:
        linear-gradient(135deg, rgba(17, 94, 89, 0.08), rgba(255, 250, 240, 0.98)),
        linear-gradient(180deg, rgba(255, 255, 255, 0.8), rgba(255, 250, 240, 0.9));
      box-shadow: var(--shadow);
    }}

    .hero::after {{
      content: "";
      position: absolute;
      inset: auto -80px -110px auto;
      width: 280px;
      height: 280px;
      border-radius: 999px;
      background: radial-gradient(circle, rgba(17, 94, 89, 0.16), transparent 68%);
      pointer-events: none;
    }}

    .eyebrow {{
      display: inline-flex;
      gap: 10px;
      align-items: center;
      margin-bottom: 14px;
      padding: 7px 12px;
      border-radius: 999px;
      background: rgba(255, 255, 255, 0.7);
      border: 1px solid rgba(17, 94, 89, 0.15);
      color: var(--accent);
      font-size: 12px;
      letter-spacing: 0.08em;
      text-transform: uppercase;
      font-weight: 700;
    }}

    h1 {{
      margin: 0 0 12px;
      font-family: "Iowan Old Style", "Palatino Linotype", serif;
      font-size: clamp(2.1rem, 3.4vw, 3.4rem);
      line-height: 1.02;
      letter-spacing: -0.03em;
    }}

    .lede {{
      max-width: 900px;
      margin: 0;
      color: var(--muted);
      font-size: 1.02rem;
      line-height: 1.6;
    }}

    .meta {{
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
      margin-top: 18px;
    }}

    .pill {{
      padding: 8px 12px;
      border-radius: 999px;
      border: 1px solid var(--line);
      background: rgba(255, 255, 255, 0.72);
      color: var(--muted);
      font-size: 0.92rem;
    }}

    .stats {{
      display: grid;
      grid-template-columns: repeat(6, minmax(0, 1fr));
      gap: 14px;
      margin-top: 20px;
    }}

    .card {{
      padding: 18px 18px 16px;
      border-radius: var(--radius);
      border: 1px solid var(--line);
      background: var(--panel);
      box-shadow: 0 10px 30px rgba(58, 40, 18, 0.07);
      backdrop-filter: blur(10px);
    }}

    .card.warn {{
      border-color: rgba(154, 52, 18, 0.18);
    }}

    .kicker {{
      margin: 0 0 8px;
      color: var(--muted);
      font-size: 0.77rem;
      letter-spacing: 0.08em;
      text-transform: uppercase;
      font-weight: 700;
    }}

    .value {{
      margin: 0;
      font-family: "Iowan Old Style", "Palatino Linotype", serif;
      font-size: clamp(1.6rem, 2.2vw, 2.1rem);
      line-height: 1;
    }}

    .note {{
      margin: 8px 0 0;
      color: var(--muted);
      font-size: 0.92rem;
      line-height: 1.45;
    }}

    .section {{
      margin-top: 22px;
      padding: 22px;
      border-radius: 24px;
      border: 1px solid var(--line);
      background: var(--panel);
      box-shadow: var(--shadow);
    }}

    h2 {{
      margin: 0 0 10px;
      font-family: "Iowan Old Style", "Palatino Linotype", serif;
      font-size: 1.7rem;
      line-height: 1.1;
    }}

    .section p {{
      margin: 0;
      color: var(--muted);
      line-height: 1.6;
    }}

    .legend {{
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
      margin-top: 16px;
    }}

    .legend-item {{
      padding: 10px 12px;
      border-radius: 12px;
      background: var(--bg-soft);
      border: 1px solid var(--line);
      font-size: 0.92rem;
      color: var(--muted);
    }}

    .table-wrap {{
      margin-top: 18px;
      overflow: auto;
      border-radius: 20px;
      border: 1px solid var(--line);
      background: var(--panel-strong);
    }}

    table {{
      width: 100%;
      border-collapse: collapse;
      min-width: 980px;
    }}

    thead th {{
      position: sticky;
      top: 0;
      z-index: 2;
      text-align: left;
      padding: 15px 16px;
      background: rgba(255, 248, 238, 0.97);
      border-bottom: 1px solid var(--line);
      color: var(--muted);
      font-size: 0.82rem;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }}

    tbody tr:nth-child(odd) {{
      background: rgba(255, 255, 255, 0.35);
    }}

    tbody tr:hover {{
      background: rgba(17, 94, 89, 0.06);
    }}

    td {{
      vertical-align: top;
      padding: 18px 16px;
      border-top: 1px solid var(--line);
    }}

    .rule-code {{
      display: inline-flex;
      gap: 8px;
      align-items: center;
      margin-bottom: 8px;
      font-size: 0.95rem;
      font-weight: 700;
    }}

    .badge {{
      padding: 5px 9px;
      border-radius: 999px;
      border: 1px solid rgba(17, 94, 89, 0.18);
      background: var(--accent-soft);
      color: var(--accent);
    }}

    .rule-desc {{
      margin: 0;
      color: var(--muted);
      line-height: 1.5;
      font-size: 0.95rem;
    }}

    .metric {{
      font-family: "Iowan Old Style", "Palatino Linotype", serif;
      font-size: 1.5rem;
      line-height: 1;
      margin: 0 0 6px;
    }}

    .metric-label {{
      margin: 0;
      color: var(--muted);
      font-size: 0.87rem;
    }}

    .reason-list {{
      margin: 0;
      padding-left: 18px;
      color: var(--text);
    }}

    .reason-list li {{
      margin: 0 0 9px;
      line-height: 1.45;
    }}

    .reason-list li:last-child {{
      margin-bottom: 0;
    }}

    .count-tag {{
      display: inline-block;
      min-width: 5.7em;
      margin-right: 8px;
      padding: 4px 8px;
      border-radius: 999px;
      background: rgba(17, 94, 89, 0.1);
      color: var(--accent);
      font-size: 0.83rem;
      font-weight: 700;
      text-align: center;
    }}

    .sc-only {{
      color: var(--warn);
      font-weight: 700;
    }}

    .shuck-only {{
      color: var(--accent);
      font-weight: 700;
    }}

    .footer {{
      margin-top: 18px;
      padding: 18px 20px;
      border-radius: 18px;
      background: rgba(255, 255, 255, 0.62);
      border: 1px solid var(--line);
      color: var(--muted);
      line-height: 1.6;
    }}

    code {{
      font-family: "SFMono-Regular", "Menlo", "Consolas", monospace;
      font-size: 0.92em;
      padding: 0.1em 0.35em;
      border-radius: 6px;
      background: rgba(17, 94, 89, 0.08);
      color: #164e63;
    }}

    .bucket-chip {{
      display: inline-flex;
      align-items: center;
      padding: 6px 10px;
      border-radius: 999px;
      border: 1px solid rgba(154, 52, 18, 0.18);
      background: rgba(154, 52, 18, 0.08);
      color: var(--warn);
      font-size: 0.84rem;
      font-weight: 700;
      white-space: nowrap;
    }}

    .fixture-name {{
      font-weight: 700;
      line-height: 1.35;
    }}

    .fixture-path {{
      margin-top: 8px;
      color: var(--muted);
      line-height: 1.45;
      word-break: break-word;
    }}

    .chip-list {{
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
    }}

    .code-chip {{
      display: inline-flex;
      align-items: center;
      padding: 6px 10px;
      border-radius: 999px;
      border: 1px solid rgba(17, 94, 89, 0.16);
      background: rgba(17, 94, 89, 0.08);
      color: var(--accent);
      font-size: 0.84rem;
      font-weight: 700;
      white-space: nowrap;
    }}

    .detail-list {{
      margin: 0;
      padding-left: 18px;
      color: var(--text);
    }}

    .detail-list-tight {{
      margin-top: 10px;
    }}

    .detail-item,
    .detail-item-muted {{
      margin: 0 0 8px;
      line-height: 1.45;
    }}

    .detail-item:last-child,
    .detail-item-muted:last-child {{
      margin-bottom: 0;
    }}

    .detail-item-muted {{
      color: var(--muted);
      font-size: 0.87rem;
    }}

    @media (max-width: 1080px) {{
      .stats {{
        grid-template-columns: repeat(3, minmax(0, 1fr));
      }}
    }}

    @media (max-width: 720px) {{
      .page {{
        width: min(100vw - 18px, 1320px);
        margin: 10px auto 24px;
      }}

      .hero,
      .section {{
        padding: 18px;
        border-radius: 22px;
      }}

      .stats {{
        grid-template-columns: repeat(2, minmax(0, 1fr));
      }}

      td,
      thead th {{
        padding: 14px 12px;
      }}
    }}
  </style>
</head>
<body>
  <main class="page">
    <section class="hero">
      <div class="eyebrow">Large Corpus Conformance Snapshot</div>
      <h1>Rule Record Counts With Grouped Failure Reasons</h1>
      <p class="lede">
        This page summarizes a large-corpus run for Shuck and folds the raw compatibility log into a
        per-rule table. Counts below cover implementation-diff records plus allowlisted known
        failures, not failing fixtures, so a single fixture can contribute more than one displayed
        record to the same rule.
      </p>
      <div class="meta">
        <div class="pill">Generated: <code>{html.escape(generated_label)}</code></div>
        <div class="pill">Source log: <code>{html.escape(str(log_path))}</code></div>
        <div class="pill">Output: <code>{html.escape(str(output_path))}</code></div>
      </div>
      <div class="stats">
        <article class="card">
          <p class="kicker">Fixtures processed</p>
          <p class="value">{format_number(main_processed_fixtures)}</p>
          <p class="note">Main compatibility run total based on the largest observed progress count in the log.</p>
        </article>
        <article class="card">
          <p class="kicker">Rule-coded records</p>
          <p class="value">{format_number(rule_records)}</p>
          <p class="note">{len(rule_summaries)} distinct rules appeared across implementation diffs and allowlisted known failures.</p>
        </article>
        <article class="card">
          <p class="kicker">SC-only records</p>
          <p class="value">{format_number(shellcheck_only)}</p>
          <p class="note">ShellCheck emitted these; Shuck did not.</p>
        </article>
        <article class="card">
          <p class="kicker">Shuck-only records</p>
          <p class="value">{format_number(shuck_only)}</p>
          <p class="note">Shuck emitted these; ShellCheck did not.</p>
        </article>
        <article class="card warn">
          <p class="kicker">Main run blockers</p>
          <p class="value">{format_number(main_blocking)}</p>
          <p class="note">Across {format_number(main_fixture_entries)} sampled fixtures, with {format_number(unsupported_shells)} unsupported shells skipped.</p>
        </article>
        <article class="card warn">
          <p class="kicker">Zsh parse blockers</p>
          <p class="value">{format_number(zsh_blocking)}</p>
          <p class="note">Across {format_number(zsh_fixture_entries)} sampled fixtures and {format_number(zsh_harness_failures)} zsh harness failures.</p>
        </article>
      </div>
    </section>

    <section class="section">
      <h2>How To Read This</h2>
      <p>
        The grouped-reason bullets show whether a rule's mismatches skew toward ShellCheck-only
        or Shuck-only records.
      </p>
      <div class="legend">
        <div class="legend-item"><span class="sc-only">SC-only</span> = ShellCheck reported it, Shuck did not.</div>
        <div class="legend-item"><span class="shuck-only">Shuck-only</span> = Shuck reported it, ShellCheck did not.</div>
        <div class="legend-item">Top 5 rules account for {top_five_share:.1f}% of all rule-coded records.</div>
        <div class="legend-item">Only hard-coded allowlisted known failures are pulled in from reviewed divergences.</div>
      </div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Rule</th>
              <th>Mismatches</th>
              <th>Fixtures</th>
              <th>Grouped Reasons</th>
            </tr>
          </thead>
          <tbody>
            {rule_rows}
          </tbody>
        </table>
      </div>
    </section>

    <section class="section">
      <h2>Other Issue Buckets</h2>
      <p>
        The rule table above covers implementation diffs plus allowlisted known failures. This log also reported {format_number(mapping_issues)}
        mapping issues, {format_number(reviewed_divergences)} reviewed divergences,
        {format_number(main_harness_warnings)} main harness warnings,
        {format_number(main_harness_failures)} main harness failures, and {panic_text}.
      </p>
      <p class="note">
        The table below stays fixture-focused on main-run issue entries. When the log includes
        fixture-level detail, it includes implementation diffs, allowlisted known failures,
        mapping issues, harness warnings, and harness failures. Other reviewed divergences stay in
        the aggregate counts above but are intentionally omitted from the detailed tables.
      </p>
      {timeout_note_html}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Bucket</th>
              <th>Fixture</th>
              <th>Records</th>
              <th>Codes</th>
              <th>Why It Matters</th>
            </tr>
          </thead>
          <tbody>
            {blocker_rows}
          </tbody>
        </table>
      </div>
    </section>

    <div class="footer">
      Rebuild this report with <code>make large-corpus-report</code> or render from an existing log
      with <code>make large-corpus-report-from-log LARGE_CORPUS_REPORT_LOG=/path/to/log</code>.
    </div>
  </main>
</body>
</html>
"""


def main() -> int:
    args = parse_args()
    repo_root = Path(args.repo_root).resolve()
    log_path = Path(args.log).resolve()
    output_path = Path(args.output).resolve()

    if not log_path.is_file():
        raise SystemExit(f"log file not found: {log_path}")

    text = log_path.read_text(encoding="utf-8")
    main_report_body = extract_main_report_body(text) or ""
    sections = extract_sections(main_report_body)
    implementation_section = sections.get("Implementation Diffs")
    mapping_section = sections.get("Mapping Issues")
    reviewed_section = sections.get("Reviewed Divergence")
    known_failure_section = filter_reviewed_divergence_section_for_known_failures(
        reviewed_section
    )
    harness_warning_section = sections.get("Harness Warnings")
    main_harness_section = sections.get("Harness Failures")
    zsh_harness_section = optional_zsh_harness_section(text)

    main_summary_match = MAIN_SUMMARY_RE.search(text)
    main_failure_match = MAIN_FAILURE_RE.search(text)
    main_timeout_note_match = MAIN_TIMEOUT_NOTE_RE.search(text)
    if not main_summary_match and not main_failure_match:
        raise SystemExit("could not find the main compatibility summary in the log")
    zsh_failure_match = ZSH_FAILURE_RE.search(text)
    main_counts_match = main_summary_match or main_failure_match
    if main_counts_match is None:
        raise SystemExit("could not determine the main compatibility counts from the log")

    rule_record_section = combine_sections(implementation_section, known_failure_section)
    rule_summaries = parse_rule_summaries(rule_record_section, repo_root)
    blocker_entries = (
        parse_blocker_entries("Implementation Diff", implementation_section)
        + parse_blocker_entries("Known Failure", known_failure_section)
        + parse_blocker_entries("Mapping Issue", mapping_section)
        + parse_blocker_entries("Harness Warning", harness_warning_section)
        + parse_blocker_entries("Harness Failure", main_harness_section)
    )
    rule_records = sum(summary.mismatches for summary in rule_summaries)
    shellcheck_only = sum(
        count
        for summary in rule_summaries
        for side, count in summary.grouped_reasons.items()
        if side == "shellcheck-only"
    )
    shuck_only = rule_records - shellcheck_only
    main_blocking = int(main_counts_match.group("blocking"))
    main_fixture_entries = int(main_counts_match.group("fixtures"))
    unsupported_shells = int(main_counts_match.group("skipped") or "0")
    mapping_issue_count = count_diagnostic_records(mapping_section)
    if mapping_issue_count == 0 and mapping_section is None and main_summary_match:
        mapping_issue_count = int(main_summary_match.group("mapping"))
    reviewed_divergence_count = count_diagnostic_records(reviewed_section)
    if (
        reviewed_divergence_count == 0
        and reviewed_section is None
        and main_summary_match
    ):
        reviewed_divergence_count = int(main_summary_match.group("reviewed"))
    main_harness_warning_count = (
        int(main_summary_match.group("harness_warnings"))
        if main_summary_match
        else count_fixture_entries(harness_warning_section)
    )
    main_harness_failure_count = (
        int(main_summary_match.group("harness_failures"))
        if main_summary_match
        else count_fixture_entries(main_harness_section)
    )

    output_path.parent.mkdir(parents=True, exist_ok=True)
    html_text = render_html(
        log_path=log_path,
        output_path=output_path,
        generated_at=datetime.now().astimezone(),
        main_blocking=main_blocking,
        main_fixture_entries=main_fixture_entries,
        unsupported_shells=unsupported_shells,
        main_processed_fixtures=main_fixture_total(
            text, main_fixture_entries, unsupported_shells
        ),
        rule_records=rule_records,
        shellcheck_only=shellcheck_only,
        shuck_only=shuck_only,
        mapping_issues=mapping_issue_count,
        reviewed_divergences=reviewed_divergence_count,
        main_harness_warnings=main_harness_warning_count,
        main_harness_failures=main_harness_failure_count,
        main_timeout_note=main_timeout_note_match.group("note")
        if main_timeout_note_match
        else None,
        zsh_blocking=int(zsh_failure_match.group("blocking")) if zsh_failure_match else 0,
        zsh_fixture_entries=int(zsh_failure_match.group("fixtures")) if zsh_failure_match else 0,
        zsh_harness_failures=count_fixture_entries(zsh_harness_section),
        top_five_share=top_rule_share(rule_summaries),
        worker_panic_info=worker_panic(text),
        blocker_entries=blocker_entries,
        rule_summaries=rule_summaries,
    )
    output_path.write_text(html_text, encoding="utf-8")
    print(output_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
