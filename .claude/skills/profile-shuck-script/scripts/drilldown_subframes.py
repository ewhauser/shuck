#!/usr/bin/env python3
"""Break down a samply profile by sub-frame within a chosen parent function.

Use this when the top-level summarizer rolls everything inside a single fat
frame (e.g. ``LinterFactsBuilder::build``) into one bucket and you need to
see which sub-frames inside it are doing the work.

For each sample whose stack contains the parent frame, this finds the
deepest ``shuck_linter::facts::*`` (or matching ``--namespace``) frame above
the parent on the call stack and attributes the sample to that sub-frame.
Samples whose only matching frame is generic library code (allocator,
hashing, memchr, etc.) are bucketed under ``<non-facts leaf>``.
"""

from __future__ import annotations

import argparse
import bisect
import collections
import gzip
import json
import re
import subprocess
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", type=Path, help="samply profile JSON or JSON.GZ")
    parser.add_argument("binary", type=Path, help="profiled binary for nm/rustfilt symbols")
    parser.add_argument(
        "--parent",
        default="shuck_linter::facts::LinterFactsBuilder::build",
        help="substring of the parent frame whose body we drill into",
    )
    parser.add_argument(
        "--namespace",
        default="shuck_linter::facts::",
        help="substring identifying meaningful sub-frame names; everything else is bucketed as <non-facts leaf>",
    )
    parser.add_argument("--limit", type=int, default=30, help="number of rows to print")
    parser.add_argument(
        "--min-percent",
        type=float,
        default=0.2,
        help="suppress rows below this percentage of total samples",
    )
    return parser.parse_args()


def load_profile(path: Path) -> dict:
    if path.suffix == ".gz":
        with gzip.open(path, "rt") as handle:
            return json.load(handle)
    return json.loads(path.read_text())


def load_symbols(binary: Path) -> tuple[list[int], list[str]]:
    output = subprocess.check_output(["nm", "-an", str(binary)], text=True, errors="ignore")
    symbols: list[tuple[int, str]] = []
    for line in output.splitlines():
        parts = line.split(maxsplit=2)
        if len(parts) != 3 or parts[1] not in {"t", "T"}:
            continue
        name = parts[2]
        if name.startswith("_") and not name.startswith("__ZN"):
            name = name[1:]
        symbols.append((int(parts[0], 16), name))
    symbols.sort()

    unique = []
    seen = set()
    for _, name in symbols:
        if name not in seen:
            seen.add(name)
            unique.append(name)
    try:
        demangled = subprocess.check_output(
            ["rustfilt"], input="\n".join(unique), text=True
        ).splitlines()
        demap = dict(zip(unique, demangled))
    except (OSError, subprocess.CalledProcessError):
        demap = {name: name for name in unique}

    return [addr for addr, _ in symbols], [demap.get(name, name) for _, name in symbols]


def symbol_for(offset: int, addrs: list[int], names: list[str]) -> str | None:
    index = bisect.bisect_right(addrs, 0x100000000 + offset) - 1
    if index < 0:
        return None
    return names[index]


def short_symbol(name: str) -> str:
    name = name.removeprefix("_")
    name = re.sub(r"<[^<>]*>", "<>", name)
    name = re.sub(r"::\{\{closure\}\}", "::{{c}}", name)
    return name


def main() -> None:
    args = parse_args()
    profile = load_profile(args.profile)
    addrs, names = load_symbols(args.binary)
    thread = profile["threads"][0]
    func_res = thread["funcTable"]["resource"]
    res_lib = thread["resourceTable"]["lib"]
    frame_func = thread["frameTable"]["func"]
    frame_addr = thread["frameTable"]["address"]

    frame_names: list[str | None] = []
    for frame, func in enumerate(frame_func):
        lib = res_lib[func_res[func]] if func_res[func] is not None else None
        offset = frame_addr[frame]
        if lib == 1 and isinstance(offset, int):
            frame_names.append(symbol_for(offset, addrs, names))
        else:
            frame_names.append(None)

    stack_prefix = thread["stackTable"]["prefix"]
    stack_frame = thread["stackTable"]["frame"]
    weights = thread["samples"].get("weight") or [1] * thread["samples"]["length"]
    stacks = thread["samples"]["stack"]

    counts: collections.Counter[str] = collections.Counter()
    leaf_examples: dict[str, collections.Counter[str]] = {}
    total = 0
    parent_total = 0
    non_leaf_label = f"<non-{args.namespace.rstrip(':')} leaf>"

    for stack, weight in zip(stacks, weights):
        if stack is None:
            continue
        total += weight
        chain: list[str] = []
        current = stack
        while current is not None:
            n = frame_names[stack_frame[current]] or ""
            chain.append(n)
            current = stack_prefix[current]

        joined = "\n".join(chain)
        if args.parent not in joined:
            continue
        parent_total += weight

        # chain[0] is the leaf; deeper frames follow. Walk leaf -> root and
        # bucket by the first matching-namespace frame seen above the parent.
        sub: str | None = None
        for name in chain:
            if args.parent in name:
                break
            if args.namespace in name:
                sub = short_symbol(name)
                break
        if sub is None:
            leaf = chain[0] if chain else ""
            sub = f"{non_leaf_label} {short_symbol(leaf)}".rstrip()
        counts[sub] += weight
        leaf = chain[0] if chain else ""
        leaf_examples.setdefault(sub, collections.Counter())[short_symbol(leaf)] += weight

    print(f"total_samples={total}")
    if total:
        print(f"parent_samples={parent_total} ({parent_total / total * 100:.1f}% of total)")
    else:
        print(f"parent_samples={parent_total}")
    print()
    print("| Rank | Sub-frame | % of total | % of parent | Top leaf |")
    print("|---:|---|---:|---:|---|")
    rank = 1
    for sub, count in counts.most_common():
        pct_total = count / total * 100 if total else 0
        pct_parent = count / parent_total * 100 if parent_total else 0
        if rank > args.limit or pct_total < args.min_percent:
            break
        leaf = leaf_examples[sub].most_common(1)[0][0]
        print(f"| {rank} | `{sub}` | {pct_total:.1f}% | {pct_parent:.1f}% | `{leaf}` |")
        rank += 1


if __name__ == "__main__":
    main()
