#!/usr/bin/env python3
"""Generate valid shell scripts and fuzz shuck through its CLI."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import random
import shlex
import subprocess
import sys
import tempfile
import textwrap
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Callable, Final

MAX_DEPTH: Final = 3
MAX_STATEMENTS: Final = 8
MAX_HEREDOC_LINES: Final = 3
MAX_GENERATION_ATTEMPTS: Final = 25

LITERALS: Final = [
    "alpha",
    "beta",
    "gamma",
    "delta",
    "file",
    "path",
    "item",
    "name",
    "count",
    "tmp",
]
VARIABLES: Final = ["foo", "bar", "baz", "count", "path", "item", "name", "value"]
COMMANDS: Final = ["echo", "printf", "true", "false", "test", "cat", "grep", "mkdir"]


@dataclass(frozen=True)
class CaseConfig:
    seed: int
    dialect: str
    profile: str
    timeout: float
    shuck_bin: str
    artifact_dir: str


@dataclass
class CommandResult:
    args: list[str]
    returncode: int | None
    stdout: str
    stderr: str
    timed_out: bool = False
    signal: int | None = None


@dataclass
class FailureReport:
    seed: int
    dialect: str
    reason: str
    original: str
    minimized: str
    failing_command: list[str]
    command_results: list[CommandResult]
    artifact_dir: str


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--shuck-bin", default="target/debug/shuck")
    parser.add_argument("--dialect", choices=("sh", "bash"), required=True)
    parser.add_argument("--profile", choices=("smoke", "full"), default="full")
    parser.add_argument("--count", type=int, default=1)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=5.0)
    parser.add_argument("--artifact-dir", default="fuzz/artifacts/cli")
    return parser.parse_args()


def run_command(
    args: list[str],
    *,
    cwd: Path,
    timeout: float,
    input_text: str | None = None,
) -> CommandResult:
    try:
        completed = subprocess.run(
            args,
            cwd=cwd,
            capture_output=True,
            text=True,
            input=input_text,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as error:
        return CommandResult(
            args=args,
            returncode=None,
            stdout=error.stdout or "",
            stderr=error.stderr or "",
            timed_out=True,
        )

    signal = -completed.returncode if completed.returncode < 0 else None
    return CommandResult(
        args=args,
        returncode=completed.returncode,
        stdout=completed.stdout,
        stderr=completed.stderr,
        signal=signal,
    )


def shell_binary(dialect: str) -> str:
    return "bash" if dialect == "bash" else "sh"


def script_extension(dialect: str) -> str:
    return ".bash" if dialect == "bash" else ".sh"


def random_literal(rng: random.Random) -> str:
    base = rng.choice(LITERALS)
    suffix = str(rng.randrange(10)) if rng.random() < 0.4 else ""
    return f"{base}{suffix}"[:12]


def indent(text: str, prefix: str = "  ") -> str:
    return "\n".join(f"{prefix}{line}" if line else prefix.rstrip() for line in text.splitlines())


def random_word(rng: random.Random, dialect: str, depth: int) -> str:
    choice = rng.randrange(6 if depth < MAX_DEPTH else 4)
    if choice == 0:
        return random_literal(rng)
    if choice == 1:
        return f"'{random_literal(rng)}'"
    if choice == 2:
        return f'"{random_literal(rng)} {random_literal(rng)}"'
    if choice == 3:
        return f"${rng.choice(VARIABLES)}"
    if choice == 4:
        return f"${{{rng.choice(VARIABLES)}}}"
    command = simple_command(rng, dialect, depth + 1, allow_redirect=False)
    return f"$({command})"


def random_smoke_word(rng: random.Random) -> str:
    choice = rng.randrange(5)
    if choice == 0:
        return random_literal(rng)
    if choice == 1:
        return f"'{random_literal(rng)}'"
    if choice == 2:
        return f'"{random_literal(rng)} {random_literal(rng)}"'
    if choice == 3:
        return f"${rng.choice(VARIABLES)}"
    return f"${{{rng.choice(VARIABLES)}}}"


def assignment(rng: random.Random, dialect: str, depth: int) -> str:
    return f"{rng.choice(VARIABLES)}={random_word(rng, dialect, depth)}"


def redirection(rng: random.Random, dialect: str, depth: int) -> str:
    target = f"/tmp/{random_literal(rng)}"
    operator = rng.choice([">", ">>", "2>", "<"])
    return f"{operator} {target}" if operator == "<" else f"{operator} {shlex.quote(target)}"


def simple_command(
    rng: random.Random, dialect: str, depth: int, *, allow_redirect: bool = True
) -> str:
    if dialect == "bash" and depth < MAX_DEPTH and rng.random() < 0.15:
        left = rng.choice(VARIABLES)
        right = random_literal(rng)
        return f"[[ ${left} == {right} ]]"

    if dialect == "bash" and depth < MAX_DEPTH and rng.random() < 0.15:
        value = rng.randrange(10)
        return f"(( {rng.choice(VARIABLES)} + {value} ))"

    name = rng.choice(COMMANDS)
    args = [random_word(rng, dialect, depth) for _ in range(rng.randrange(0, 4))]
    pieces = [name, *args]

    if rng.random() < 0.25:
        pieces.insert(0, assignment(rng, dialect, depth))
    if allow_redirect and rng.random() < 0.25:
        pieces.append(redirection(rng, dialect, depth))

    return " ".join(pieces)


def simple_smoke_command(rng: random.Random) -> str:
    if rng.random() < 0.35:
        return f"{rng.choice(VARIABLES)}={random_smoke_word(rng)}"

    name = rng.choice(["echo", "true", "false", "test"])
    args = [random_smoke_word(rng) for _ in range(rng.randrange(0, 3))]
    return " ".join([name, *args]).strip()


def compound_if(rng: random.Random, dialect: str, depth: int, profile: str) -> str:
    condition = simple_command(rng, dialect, depth + 1, allow_redirect=False)
    then_block = block(rng, dialect, depth + 1, profile)
    if rng.random() < 0.5:
        else_block = block(rng, dialect, depth + 1, profile)
        return f"if {condition}; then\n{indent(then_block)}\nelse\n{indent(else_block)}\nfi"
    return f"if {condition}; then\n{indent(then_block)}\nfi"


def compound_while(rng: random.Random, dialect: str, depth: int, profile: str) -> str:
    condition = simple_command(rng, dialect, depth + 1, allow_redirect=False)
    body = block(rng, dialect, depth + 1, profile)
    keyword = "until" if rng.random() < 0.3 else "while"
    return f"{keyword} {condition}; do\n{indent(body)}\ndone"


def compound_for(rng: random.Random, dialect: str, depth: int, profile: str) -> str:
    items = " ".join(random_word(rng, dialect, depth + 1) for _ in range(rng.randrange(1, 4)))
    body = block(rng, dialect, depth + 1, profile)
    return f"for {rng.choice(VARIABLES)} in {items}; do\n{indent(body)}\ndone"


def compound_case(rng: random.Random, dialect: str, depth: int, profile: str) -> str:
    subject = random_word(rng, dialect, depth + 1)
    first_pattern = f"{random_literal(rng)}*"
    second_pattern = "*"
    first_body = indent(block(rng, dialect, depth + 1, profile))
    second_body = indent(block(rng, dialect, depth + 1, profile))
    return (
        f"case {subject} in\n"
        f"  {first_pattern})\n{first_body}\n    ;;\n"
        f"  {second_pattern})\n{second_body}\n    ;;\n"
        f"esac"
    )


def compound_function(rng: random.Random, dialect: str, depth: int, profile: str) -> str:
    name = f"{random_literal(rng)}_{rng.randrange(10)}"
    body = block(rng, dialect, depth + 1, profile)
    return f"{name}() {{\n{indent(body)}\n}}"


def compound_subshell(rng: random.Random, dialect: str, depth: int, profile: str) -> str:
    return f"(\n{indent(block(rng, dialect, depth + 1, profile))}\n)"


def heredoc(rng: random.Random, dialect: str, depth: int) -> str:
    marker = f"EOF_{rng.randrange(1000)}"
    lines = [random_literal(rng) for _ in range(rng.randrange(1, MAX_HEREDOC_LINES + 1))]
    payload = "\n".join(lines)
    return f"cat <<'{marker}'\n{payload}\n{marker}"


def statement(rng: random.Random, dialect: str, depth: int, profile: str) -> str:
    if profile == "smoke":
        return simple_smoke_command(rng)

    if depth >= MAX_DEPTH:
        return simple_command(rng, dialect, depth)

    builders: list[Callable[[random.Random, str, int, str], str]] = [
        lambda rng, dialect, depth, profile: simple_command(rng, dialect, depth),
        compound_if,
        compound_while,
        compound_for,
        compound_case,
        compound_function,
        compound_subshell,
    ]
    if profile == "full":
        builders.append(lambda rng, dialect, depth, profile: heredoc(rng, dialect, depth))
    return rng.choice(builders)(rng, dialect, depth, profile)


def block(rng: random.Random, dialect: str, depth: int, profile: str) -> str:
    count = rng.randrange(1, 4)
    return "\n".join(statement(rng, dialect, depth, profile) for _ in range(count))


def generate_script(dialect: str, seed: int, profile: str) -> str:
    for attempt in range(MAX_GENERATION_ATTEMPTS):
        rng = random.Random(seed + attempt * 1_000_003)
        statements = [
            statement(rng, dialect, 0, profile) for _ in range(rng.randrange(1, MAX_STATEMENTS + 1))
        ]
        shebang = "#!/usr/bin/env bash" if dialect == "bash" else "#!/bin/sh"
        script = f"{shebang}\n" + "\n".join(statements).strip() + "\n"
        if validate_script(script, dialect):
            return script
    return fallback_script(dialect, seed)


def fallback_script(dialect: str, seed: int) -> str:
    shebang = "#!/usr/bin/env bash" if dialect == "bash" else "#!/bin/sh"
    value = seed % 10
    if dialect == "bash":
        body = textwrap.dedent(
            f"""\
            count={value}
            (( count += 1 ))
            printf '%s\\n' "$count"
            """
        )
    else:
        body = textwrap.dedent(
            f"""\
            count={value}
            count=$((count + 1))
            printf '%s\\n' "$count"
            """
        )
    return f"{shebang}\n{body}"


def validate_script(script: str, dialect: str) -> bool:
    with tempfile.TemporaryDirectory() as tempdir:
        path = Path(tempdir, f"validate{script_extension(dialect)}")
        path.write_text(script)
        result = subprocess.run(
            [shell_binary(dialect), "-n", str(path)],
            capture_output=True,
            text=True,
        )
        return result.returncode == 0


def evaluate_bug(
    script: str, config: CaseConfig
) -> tuple[bool, str, list[CommandResult], list[str]]:
    with tempfile.TemporaryDirectory() as tempdir:
        script_path = Path(tempdir, f"generated{script_extension(config.dialect)}")
        script_path.write_text(script)

        if not validate_script(script, config.dialect):
            return True, "generator produced invalid shell syntax", [], [config.shuck_bin]

        commands: list[CommandResult] = []
        check_result = run_command(
            [config.shuck_bin, "check", "--no-cache", str(script_path)],
            cwd=Path.cwd(),
            timeout=config.timeout,
        )
        commands.append(check_result)
        bug = classify_command_failure(check_result, allowed_returncodes={0, 1})
        if bug:
            return True, f"check failed: {bug}", commands, check_result.args

        format_args = [
            config.shuck_bin,
            "format",
            "--no-cache",
            "--stdin-filename",
            str(script_path),
            "-",
        ]
        format_result = run_command(
            format_args,
            cwd=Path.cwd(),
            timeout=config.timeout,
            input_text=script,
        )
        commands.append(format_result)
        bug = classify_command_failure(format_result, allowed_returncodes={0})
        if bug:
            return True, f"format failed: {bug}", commands, format_result.args

        formatted = format_result.stdout

        reformatted_result = run_command(
            format_args,
            cwd=Path.cwd(),
            timeout=config.timeout,
            input_text=formatted,
        )
        commands.append(reformatted_result)
        bug = classify_command_failure(reformatted_result, allowed_returncodes={0})
        if bug:
            return True, f"second format failed: {bug}", commands, reformatted_result.args

        if formatted != reformatted_result.stdout:
            return True, "formatter was not idempotent", commands, reformatted_result.args

        format_check_result = run_command(
            [
                config.shuck_bin,
                "format",
                "--no-cache",
                "--check",
                "--stdin-filename",
                str(script_path),
                "-",
            ],
            cwd=Path.cwd(),
            timeout=config.timeout,
            input_text=reformatted_result.stdout,
        )
        commands.append(format_check_result)
        bug = classify_command_failure(format_check_result, allowed_returncodes={0})
        if bug:
            return True, f"formatted output failed --check: {bug}", commands, format_check_result.args

    return False, "", commands, [config.shuck_bin]


def classify_command_failure(result: CommandResult, *, allowed_returncodes: set[int]) -> str | None:
    if result.timed_out:
        return "timed out"
    if result.signal is not None:
        return f"terminated by signal {result.signal}"
    if result.returncode is None:
        return "did not report an exit code"
    if result.returncode not in allowed_returncodes:
        return f"exit code {result.returncode}"
    if "panicked at" in result.stderr or "panicked at" in result.stdout:
        return "panic detected"
    return None


def minimize_script(
    script: str,
    predicate: Callable[[str], bool],
) -> str:
    candidate = script
    candidate = minimize_by_line_blocks(candidate, predicate)
    candidate = minimize_by_byte_chunks(candidate, predicate)
    return candidate


def minimize_by_line_blocks(script: str, predicate: Callable[[str], bool]) -> str:
    lines = script.splitlines(keepends=True)
    if len(lines) <= 1:
        return script

    changed = True
    while changed:
        changed = False
        for block_size in range(len(lines) - 1, 0, -1):
            for start in range(1, len(lines) - block_size + 1):
                candidate_lines = lines[:start] + lines[start + block_size :]
                candidate = "".join(candidate_lines)
                if candidate and candidate.endswith("\n") and predicate(candidate):
                    lines = candidate_lines
                    changed = True
                    break
            if changed:
                break
    return "".join(lines)


def minimize_by_byte_chunks(script: str, predicate: Callable[[str], bool]) -> str:
    candidate = script
    chunk_size = max(len(candidate) // 2, 1)
    while chunk_size >= 1:
        changed = False
        start = 0
        while start < len(candidate):
            end = min(start + chunk_size, len(candidate))
            trial = candidate[:start] + candidate[end:]
            if trial and trial.endswith("\n") and predicate(trial):
                candidate = trial
                changed = True
                start = 0
                continue
            start += chunk_size
        if not changed:
            chunk_size //= 2
    return candidate


def save_failure(report: FailureReport) -> Path:
    artifact_root = Path(report.artifact_dir)
    artifact_root.mkdir(parents=True, exist_ok=True)
    case_dir = unique_case_dir(artifact_root, report.dialect, report.seed)
    case_dir.mkdir(parents=True, exist_ok=False)

    repro_path = case_dir / f"repro{script_extension(report.dialect)}"
    original_path = case_dir / f"original{script_extension(report.dialect)}"
    metadata_path = case_dir / "metadata.json"
    instructions_path = case_dir / "README.txt"

    repro_path.write_text(report.minimized)
    original_path.write_text(report.original)
    metadata_path.write_text(
        json.dumps(
            {
                "seed": report.seed,
                "dialect": report.dialect,
                "reason": report.reason,
                "failing_command": report.failing_command,
                "commands": [asdict(result) for result in report.command_results],
            },
            indent=2,
        )
        + "\n"
    )
    instructions_path.write_text(
        textwrap.dedent(
            f"""\
            Reproduction
            ============

            Seed: {report.seed}
            Dialect: {report.dialect}
            Reason: {report.reason}

            Validate syntax:
              {shell_binary(report.dialect)} -n {repro_path.name}

            Run check:
              {shlex.join([report.failing_command[0], 'check', '--no-cache', repro_path.name])}

            Run format:
              {shlex.join([report.failing_command[0], 'format', '--no-cache', '--stdin-filename', repro_path.name, '-'])} < {repro_path.name}
            """
        )
    )
    return case_dir


def unique_case_dir(root: Path, dialect: str, seed: int) -> Path:
    stem = f"bug-{dialect}-{seed}"
    candidate = root / stem
    suffix = 1
    while candidate.exists():
        candidate = root / f"{stem}-{suffix}"
        suffix += 1
    return candidate


def fuzz_case(config: CaseConfig) -> FailureReport | None:
    script = generate_script(config.dialect, config.seed, config.profile)
    bug_found, reason, command_results, failing_command = evaluate_bug(script, config)
    if not bug_found:
        return None

    def predicate(candidate: str) -> bool:
        if not validate_script(candidate, config.dialect):
            return False
        has_bug, _, _, _ = evaluate_bug(candidate, config)
        return has_bug

    minimized = minimize_script(script, predicate)
    report = FailureReport(
        seed=config.seed,
        dialect=config.dialect,
        reason=reason,
        original=script,
        minimized=minimized,
        failing_command=failing_command,
        command_results=command_results,
        artifact_dir=config.artifact_dir,
    )
    case_dir = save_failure(report)
    report.artifact_dir = str(case_dir)
    return report


def run_cases(configs: list[CaseConfig], workers: int) -> list[FailureReport]:
    failures: list[FailureReport] = []
    if workers <= 1:
        for index, config in enumerate(configs, start=1):
            report = fuzz_case(config)
            print_progress(index, len(configs), config, report)
            if report is not None:
                failures.append(report)
        return failures

    with concurrent.futures.ProcessPoolExecutor(max_workers=workers) as executor:
        futures = {executor.submit(fuzz_case, config): config for config in configs}
        for index, future in enumerate(concurrent.futures.as_completed(futures), start=1):
            config = futures[future]
            report = future.result()
            print_progress(index, len(configs), config, report)
            if report is not None:
                failures.append(report)
    return failures


def print_progress(index: int, total: int, config: CaseConfig, report: FailureReport | None) -> None:
    status = "BUG" if report is not None else "ok"
    print(
        f"[{index}/{total}] {config.dialect} seed={config.seed} {status}",
        flush=True,
    )
    if report is not None:
        print(f"  reason: {report.reason}", flush=True)
        print(f"  artifact: {report.artifact_dir}", flush=True)


def main() -> int:
    args = parse_args()
    shuck_bin = Path(args.shuck_bin)
    if not shuck_bin.exists():
        print(f"shuck binary not found: {shuck_bin}", file=sys.stderr)
        return 2

    configs = [
        CaseConfig(
            seed=args.seed + offset,
            dialect=args.dialect,
            profile=args.profile,
            timeout=args.timeout,
            shuck_bin=str(shuck_bin.resolve()),
            artifact_dir=args.artifact_dir,
        )
        for offset in range(args.count)
    ]

    start = time.time()
    failures = run_cases(configs, args.workers)
    duration = time.time() - start

    print(
        f"Completed {len(configs)} cases in {duration:.1f}s with {len(failures)} failure(s).",
        flush=True,
    )
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
