#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

VALID_LEVELS = {"error", "warning", "info", "style"}
SUPPORTED_SHELLS = {"sh", "bash", "dash", "ksh", "busybox", "zsh"}
SHELLCHECK_CODE_RE = re.compile(r"^SC(?P<number>\d+)$")
SHELLCHECK_LEVEL_LINE_RE = re.compile(r"^shellcheck_level:\s*.*$", re.MULTILINE)
SHELLCHECK_CODE_LINE_RE = re.compile(r"^(shellcheck_code:\s*.*)$", re.MULTILINE)
SHELLCHECK_CODE_VALUE_RE = re.compile(r"^shellcheck_code:\s*(?P<value>\S.*?)\s*$", re.MULTILINE)
RUNTIME_KIND_LINE_RE = re.compile(r"^(runtime_kind:\s*.*)$", re.MULTILINE)
TOP_LEVEL_KEY_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*:\s*")
EXAMPLE_KIND_RE = re.compile(r"^  - kind:\s*(?P<kind>\S.*?)\s*$")
SUCCESSOR_SHELLCHECK_CODES: dict[str, tuple[int, ...]] = {
    # Current ShellCheck emits the substitution-level warning at SC2327 while
    # still attaching a nested redirection error at SC2328. Shuck reports the
    # outer command-substitution span, so SC2327 is the level-compatible oracle.
    "C057": (2327,),
    "C058": (2327,),
    # Current ShellCheck reports empty else bodies at SC1048 on the same span
    # as Shuck's dangling-else diagnostic.
    "C143": (1048,),
}
MANUAL_SHELLCHECK_LEVELS: dict[str, str] = {
    # Current ShellCheck emits two diagnostics on the exact `*/` span: a
    # warning for running a glob as a command and an error for the slash-ended
    # command name. When forced to pick a single compatibility level, prefer
    # the higher-severity oracle result on the same span.
    "C054": "error",
    # C109 is intentionally kept as a historical compatibility alias after
    # current ShellCheck stopped flagging this pattern. Preserve its previous
    # compatibility intent at warning level.
    "C109": "warning",
}


@dataclass(frozen=True)
class RuleMetadata:
    path: Path
    shellcheck_code: str | None
    shells: list[str]
    invalid_example: str | None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Populate docs/rules/*.yaml with shellcheck_level by running each "
            "rule's invalid example through ShellCheck's json1 oracle."
        )
    )
    parser.add_argument(
        "--rules",
        help="Comma-separated list of rule codes to update, for example C001,C006",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Verify the current metadata instead of writing updates",
    )
    parser.add_argument(
        "--shellcheck-bin",
        default=os.environ.get("SHUCK_SHELLCHECK_BIN", "shellcheck"),
        help="ShellCheck executable to invoke (default: %(default)s)",
    )
    parser.add_argument(
        "--rules-dir",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "docs" / "rules",
        help="Directory containing rule YAML files (default: %(default)s)",
    )
    return parser.parse_args()


def load_rule_metadata(path: Path) -> RuleMetadata:
    text = path.read_text()

    shellcheck_code_match = SHELLCHECK_CODE_VALUE_RE.search(text)
    shellcheck_code = None
    if shellcheck_code_match is not None:
        shellcheck_code = shellcheck_code_match.group("value")
        if shellcheck_code in {"null", "~", ""}:
            shellcheck_code = None

    lines = text.splitlines(keepends=True)
    shells = parse_shells(lines)
    invalid_example = parse_first_invalid_example(lines)

    return RuleMetadata(
        path=path,
        shellcheck_code=shellcheck_code,
        shells=shells,
        invalid_example=invalid_example,
    )


def parse_shells(lines: list[str]) -> list[str]:
    shells: list[str] = []
    for index, line in enumerate(lines):
        if line.rstrip("\n") != "shells:":
            continue

        for shell_line in lines[index + 1 :]:
            if shell_line.startswith("  - "):
                shells.append(shell_line[4:].strip())
                continue
            if shell_line.startswith("  "):
                continue
            break
        break
    return shells


def parse_first_invalid_example(lines: list[str]) -> str | None:
    in_examples = False
    index = 0

    while index < len(lines):
        line = lines[index]
        stripped = line.rstrip("\n")

        if not in_examples:
            if stripped == "examples:":
                in_examples = True
            index += 1
            continue

        if TOP_LEVEL_KEY_RE.match(stripped):
            break

        kind_match = EXAMPLE_KIND_RE.match(stripped)
        if kind_match is None:
            index += 1
            continue

        index += 1
        if kind_match.group("kind") != "invalid":
            continue

        while index < len(lines):
            candidate = lines[index]
            candidate_stripped = candidate.rstrip("\n")

            if candidate_stripped == "    code: |":
                index += 1
                code_lines: list[str] = []
                while index < len(lines):
                    code_line = lines[index]
                    if code_line.startswith("      "):
                        code_lines.append(code_line[6:])
                        index += 1
                        continue
                    if code_line == "\n":
                        code_lines.append(code_line)
                        index += 1
                        continue
                    break
                return "".join(code_lines)

            if candidate_stripped.startswith('    code: "'):
                raw_value = candidate_stripped[len("    code: ") :]
                return json.loads(raw_value)

            if EXAMPLE_KIND_RE.match(candidate_stripped) or TOP_LEVEL_KEY_RE.match(candidate_stripped):
                return None

            index += 1

        return None

    return None


def parse_shellcheck_number(shellcheck_code: str) -> int:
    match = SHELLCHECK_CODE_RE.fullmatch(shellcheck_code.strip())
    if match is None:
        raise RuntimeError(f"invalid shellcheck_code {shellcheck_code!r}")
    return int(match.group("number"))


def shell_from_shebang(example: str) -> str | None:
    first_line = example.splitlines()[0].strip() if example.splitlines() else ""
    if not first_line.startswith("#!"):
        return None

    tokens = first_line[2:].strip().split()
    if not tokens:
        return None

    head = Path(tokens[0]).name
    if head == "env":
        candidates = [token for token in tokens[1:] if token != "-S"]
    else:
        candidates = [head, *tokens[1:]]

    for token in candidates:
        candidate = Path(token).name.lower()
        if candidate in SUPPORTED_SHELLS:
            return candidate

    return None


def detect_shell(rule: RuleMetadata) -> str:
    if rule.invalid_example:
        detected = shell_from_shebang(rule.invalid_example)
        if detected is not None:
            return detected

    for shell in rule.shells:
        if shell in SUPPORTED_SHELLS:
            return shell

    raise RuntimeError(f"{rule.path.name}: could not determine shell for invalid example")


def shellcheck_level_for_rule(rule: RuleMetadata, shellcheck_bin: str) -> str | None:
    manual_level = MANUAL_SHELLCHECK_LEVELS.get(rule.path.stem)
    if manual_level is not None:
        print(
            f"note: {rule.path.name}: using configured ShellCheck level {manual_level}",
            file=sys.stderr,
        )
        return manual_level

    if rule.shellcheck_code is None:
        return None
    if not rule.invalid_example:
        print(
            f"note: {rule.path.name}: no invalid example is available; leaving "
            "shellcheck_level as null",
            file=sys.stderr,
        )
        return None

    shellcheck_number = parse_shellcheck_number(rule.shellcheck_code)
    shell = detect_shell(rule)

    with tempfile.NamedTemporaryFile(
        mode="w",
        prefix=f"{rule.path.stem.lower()}-",
        suffix=".sh",
        delete=False,
    ) as handle:
        handle.write(rule.invalid_example)
        temp_path = Path(handle.name)

    try:
        completed = subprocess.run(
            [
                shellcheck_bin,
                "--norc",
                "-s",
                shell,
                "-f",
                "json1",
                str(temp_path),
            ],
            check=False,
            capture_output=True,
            text=True,
        )
    finally:
        temp_path.unlink(missing_ok=True)

    if completed.returncode not in {0, 1}:
        stderr = completed.stderr.strip()
        stdout = completed.stdout.strip()
        details = stderr or stdout or f"exit {completed.returncode}"
        raise RuntimeError(f"{rule.path.name}: shellcheck failed: {details}")

    if not completed.stdout.strip():
        print(
            f"note: {rule.path.name}: shellcheck produced no json1 output; leaving "
            "shellcheck_level as null",
            file=sys.stderr,
        )
        return None

    payload = json.loads(completed.stdout)
    comments = payload["comments"] if isinstance(payload, dict) else payload
    if not comments:
        print(
            f"note: {rule.path.name}: the invalid example did not produce any ShellCheck "
            "diagnostics; leaving shellcheck_level as null",
            file=sys.stderr,
        )
        return None

    matching_levels = sorted(
        {
            comment["level"]
            for comment in comments
            if int(comment.get("code", -1)) == shellcheck_number
        }
    )

    if matching_levels:
        levels = matching_levels
    else:
        successor_codes = SUCCESSOR_SHELLCHECK_CODES.get(rule.path.stem, ())
        successor_levels = sorted(
            {
                comment["level"]
                for comment in comments
                if int(comment.get("code", -1)) in successor_codes
            }
        )
        if successor_levels:
            if len(successor_levels) != 1:
                raise RuntimeError(
                    f"{rule.path.name}: successor ShellCheck codes {successor_codes} matched "
                    f"multiple levels {successor_levels}"
                )
            levels = successor_levels
            print(
                f"note: {rule.path.name}: {rule.shellcheck_code} was not present in the invalid "
                f"example output; using successor code(s) {list(successor_codes)} with level "
                f"{levels[0]}",
                file=sys.stderr,
            )
        else:
            fallback_levels = sorted({comment["level"] for comment in comments})
            seen_codes = sorted({int(comment.get("code", -1)) for comment in comments})
            if len(fallback_levels) != 1:
                print(
                    f"note: {rule.path.name}: {rule.shellcheck_code} was not present in the invalid "
                    f"example output and the remaining codes {seen_codes} had conflicting levels "
                    f"{fallback_levels}; leaving shellcheck_level as null",
                    file=sys.stderr,
                )
                return None

            levels = fallback_levels
            print(
                f"note: {rule.path.name}: {rule.shellcheck_code} was not present in the invalid "
                f"example output; using shared ShellCheck level {levels[0]} from codes {seen_codes}",
                file=sys.stderr,
            )

    if len(levels) != 1:
        raise RuntimeError(
            f"{rule.path.name}: expected one ShellCheck level for {rule.shellcheck_code}, "
            f"saw {levels}"
        )

    level = levels[0]
    if level not in VALID_LEVELS:
        raise RuntimeError(f"{rule.path.name}: unexpected ShellCheck level {level!r}")
    return level


def render_shellcheck_level(level: str | None) -> str:
    return level if level is not None else "null"


def rewrite_shellcheck_level(text: str, level: str | None) -> str:
    new_line = f"shellcheck_level: {render_shellcheck_level(level)}"

    if SHELLCHECK_LEVEL_LINE_RE.search(text):
        return SHELLCHECK_LEVEL_LINE_RE.sub(new_line, text, count=1)

    if SHELLCHECK_CODE_LINE_RE.search(text):
        return SHELLCHECK_CODE_LINE_RE.sub(rf"\1\n{new_line}", text, count=1)

    if RUNTIME_KIND_LINE_RE.search(text):
        return RUNTIME_KIND_LINE_RE.sub(
            rf"\1\nshellcheck_code: null\n{new_line}",
            text,
            count=1,
        )

    raise RuntimeError("missing runtime_kind field")


def selected_rule_paths(rules_dir: Path, selected_codes: set[str] | None) -> list[Path]:
    paths = sorted(
        path
        for path in rules_dir.glob("*.yaml")
        if path.name != "validate.sh" and (selected_codes is None or path.stem in selected_codes)
    )
    if selected_codes is not None:
        missing = selected_codes - {path.stem for path in paths}
        if missing:
            missing_csv = ", ".join(sorted(missing))
            raise RuntimeError(f"unknown rule code(s): {missing_csv}")
    return paths


def parse_selected_codes(raw: str | None) -> set[str] | None:
    if raw is None:
        return None

    codes = {
        part.strip().upper()
        for part in raw.split(",")
        if part.strip()
    }
    return codes or None


def main() -> int:
    args = parse_args()
    selected_codes = parse_selected_codes(args.rules)
    rules_dir = args.rules_dir.resolve()

    try:
        rule_paths = selected_rule_paths(rules_dir, selected_codes)
        planned_levels: dict[Path, str | None] = {}
        stale_rules: list[str] = []

        for path in rule_paths:
            metadata = load_rule_metadata(path)
            level = shellcheck_level_for_rule(metadata, args.shellcheck_bin)
            planned_levels[path] = level

        for path, level in planned_levels.items():
            current_text = path.read_text()
            updated_text = rewrite_shellcheck_level(current_text, level)
            if current_text != updated_text:
                stale_rules.append(path.stem)
                if not args.check:
                    path.write_text(updated_text)
    except RuntimeError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1

    if args.check:
        if stale_rules:
            print(
                "shellcheck levels are out of date for: " + ", ".join(sorted(stale_rules)),
                file=sys.stderr,
            )
            return 1
        print(f"verified {len(planned_levels)} rule files")
        return 0

    print(f"updated {len(rule_paths)} rule files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
