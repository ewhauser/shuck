#!/usr/bin/env python3

"""Filter large-corpus test output down to the high-signal lines for CI logs."""

from __future__ import annotations

import re
import sys


SECTION_HEADERS = {
    "Implementation Diffs:",
    "Mapping Issues:",
    "Reviewed Divergence:",
    "Corpus Noise:",
    "Harness Warnings:",
    "Harness Failures:",
}
MAX_BLOCKS_PER_SECTION = 8
FIXTURE_HEADER_RE = re.compile(r"^/.*$")
IMPORTANT_LINE_RES = [
    re.compile(r"^running \d+ tests$"),
    re.compile(r"^large corpus: processed \d+/\d+ fixtures"),
    re.compile(r"^large corpus compatibility summary: "),
    re.compile(r"^large corpus (compatibility|zsh parse) note: "),
    re.compile(r"^large corpus .* had \d+ blocking issue\(s\) "),
    re.compile(r"^large corpus test skipped "),
    re.compile(r"^large corpus zsh parse skipped "),
    re.compile(r"^thread 'large_corpus"),
    re.compile(r"^test large_corpus_"),
    re.compile(r"^failures:$"),
    re.compile(r"^test result: "),
    re.compile(r"^error: test failed"),
    re.compile(r"^make: \*\*\*"),
    re.compile(r"^Nonblocking issue buckets were omitted "),
]


class SectionState:
    def __init__(self, name: str) -> None:
        self.name = name
        self.printed_blocks = 0
        self.suppressed_blocks = 0
        self.current_block: list[str] = []


def should_print_line(line: str) -> bool:
    stripped = line.rstrip("\n")
    if stripped in SECTION_HEADERS:
        return True
    return any(regex.search(stripped) for regex in IMPORTANT_LINE_RES)


def flush_block(state: SectionState) -> list[str]:
    if not state.current_block:
        return []

    block = state.current_block
    state.current_block = []

    if state.printed_blocks < MAX_BLOCKS_PER_SECTION:
        state.printed_blocks += 1
        return block + ["\n"]

    state.suppressed_blocks += 1
    return []


def finish_section(state: SectionState) -> list[str]:
    output = flush_block(state)
    if state.suppressed_blocks > 0:
        output.append(
            f"... omitted {state.suppressed_blocks} additional entries from {state.name}\n"
        )
    return output


def iter_compacted_lines(lines: list[str]) -> list[str]:
    output: list[str] = []
    section: SectionState | None = None
    index = 0

    while index < len(lines):
        line = lines[index]
        stripped = line.rstrip("\n")

        if section is not None:
            section_end = (
                stripped in SECTION_HEADERS
                or should_print_line(line)
                and stripped not in {"", section.name}
            )
            if section_end:
                output.extend(finish_section(section))
                section = None
                continue

            if FIXTURE_HEADER_RE.match(stripped):
                output.extend(flush_block(section))
                section.current_block = [line]
            else:
                if stripped == "" and not section.current_block:
                    index += 1
                    continue
                section.current_block.append(line)
            index += 1
            continue

        if stripped in SECTION_HEADERS:
            output.append(line)
            section = SectionState(stripped[:-1])
        elif should_print_line(line):
            output.append(line)

        index += 1

    if section is not None:
        output.extend(finish_section(section))

    return output


def main() -> int:
    compacted = iter_compacted_lines(sys.stdin.readlines())
    sys.stdout.writelines(compacted)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
