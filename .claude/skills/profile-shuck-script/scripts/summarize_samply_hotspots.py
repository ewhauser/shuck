#!/usr/bin/env python3
"""Summarize a samply profile as attributed-exclusive shuck hotspots."""

from __future__ import annotations

import argparse
import bisect
import collections
import gzip
import json
import subprocess
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", type=Path, help="samply profile JSON or JSON.GZ")
    parser.add_argument("binary", type=Path, help="profiled binary for nm/rustfilt symbols")
    parser.add_argument("--limit", type=int, default=12, help="number of rows to print")
    parser.add_argument("--min-percent", type=float, default=0.3)
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
            ["rustfilt"],
            input="\n".join(unique),
            text=True,
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


def clean(name: str | None) -> str:
    return (name or "<system/unknown>").removeprefix("_")


def bucket(chain: list[str | None]) -> tuple[str, str, str]:
    names = [clean(name) for name in chain]
    stack = "\n".join(names)

    def has(text: str) -> bool:
        return text in stack

    if has("large_corpus_profile::read_fixture_manifest"):
        return ("Fixture manifest load (pre-loop)", "large_corpus_profile::read_fixture_manifest", "Setup artifact if it is large")
    if has("large_corpus_profile::run_large_corpus_fixture") and (
        has("std::fs::read_to_string") or has("std::fs::read::inner") or has("File::open_c")
    ):
        return ("Target fixture source read", "run_large_corpus_fixture -> fs::read_to_string", "Expected small per-iteration I/O")
    if has("large_corpus_profile::LargeCorpusPathResolver::new"):
        return ("Resolver map build (pre-loop)", "LargeCorpusPathResolver::new", "Should be outside sampled loop")
    if has("shuck_linter::facts::LinterFactsBuilder::build"):
        return ("Linter facts builder", "facts::LinterFactsBuilder::build", "Broad fact-building cost")
    if has("possible_variable_misspelling"):
        return ("Possible-variable-misspelling rule", "rules::possible_variable_misspelling", "Rule-local matching work")
    if has("shuck_semantic::dataflow::analyze_unused_assignments_exact"):
        return ("Unused-assignment dataflow", "semantic::dataflow::analyze_unused_assignments_exact", "Semantic dataflow")
    if has("shuck_semantic::SemanticAnalysis::uninitialized_references"):
        return ("Uninitialized reference analysis", "semantic::SemanticAnalysis::uninitialized_references", "Lazy semantic analysis")
    if has("shuck_linter::rules::correctness::local_cross_reference::local_cross_reference"):
        return ("C064 local cross-reference rule", "rules::local_cross_reference", "Iterator-heavy rule path")
    if has("shuck_semantic::builder::source_line"):
        return ("Semantic source-line lookup", "semantic::builder::source_line", "Line lookup/memchr work")
    if has("shuck_semantic::SemanticModel::scope_at"):
        return ("Scope lookup", "semantic::SemanticModel::scope_at", "Scope lookup")
    if has("shuck_semantic::build_semantic_model_base"):
        return ("Semantic model base build", "semantic::build_semantic_model_base", "Core semantic construction")
    if has("shuck_semantic::dataflow::compute_initialized_name_states_dense_with_extra_name_gens"):
        return ("Initialized-name state dataflow", "semantic::dataflow::compute_initialized_name_states_dense_with_extra_name_gens", "Semantic dataflow")
    if has("shuck_semantic::reachable_blocks_for_binding"):
        return ("CFG reachability for bindings", "semantic::reachable_blocks_for_binding", "CFG query work")
    if has("shuck_linter::facts::StaticCasePatternMatcher::advance"):
        return ("Static case-pattern matcher", "facts::StaticCasePatternMatcher::advance", "Case-pattern matching")
    if has("shuck_linter::facts::build_innermost_command_ids_by_offset"):
        return ("Innermost-command offset index", "facts::build_innermost_command_ids_by_offset", "Fact index construction")
    if has("shuck_linter::rules::correctness::script_scope_local::local_top_level"):
        return ("Script-scope-local rule", "rules::script_scope_local::local_top_level", "Rule-local scan")
    if has("shuck_semantic::build_call_graph"):
        return ("Call graph build", "semantic::build_call_graph", "Call graph construction")
    if has("array_to_string_conversion"):
        return ("Array-to-string rule", "rules::array_to_string_conversion", "Rule-local lookup")
    if has("overwritten_function"):
        return ("C063 overwritten-function rule", "rules::overwritten_function", "C063 compatibility logic")
    if has("std::sys::fs::unix::canonicalize") or has("std::sys::fs::canonicalize"):
        return ("Path canonicalization", "std::sys::fs::unix::canonicalize", "Suspicious if large; check setup capture")
    if has("std::sys::fs::unix::File::open_c"):
        return ("Other file opens", "std::sys::fs::unix::File::open_c", "Split source reads from setup")
    if has("std::fs::read::inner") or has("std::io::default_read_to_end"):
        return ("Other file reads", "std::fs::read / read_to_end", "Split source reads from setup")
    if has("alloc::raw_vec::RawVecInner"):
        return ("Vector growth/allocation", "alloc::raw_vec growth", "Allocation/growth")

    leaf = names[0] if names else "<system/unknown>"
    return (leaf, leaf, "")


def profile_frame_names(profile: dict, addrs: list[int], names: list[str]) -> tuple[dict, list[str | None]]:
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
    return thread, frame_names


def main() -> None:
    args = parse_args()
    profile = load_profile(args.profile)
    addrs, names = load_symbols(args.binary)
    thread, frame_names = profile_frame_names(profile, addrs, names)

    stack_prefix = thread["stackTable"]["prefix"]
    stack_frame = thread["stackTable"]["frame"]
    weights = thread["samples"].get("weight") or [1] * thread["samples"]["length"]
    stacks = thread["samples"]["stack"]

    counts: collections.Counter[str] = collections.Counter()
    examples: dict[str, tuple[str, str]] = {}
    total = 0
    for stack, weight in zip(stacks, weights):
        if stack is None:
            continue
        total += weight
        chain: list[str | None] = []
        current = stack
        while current is not None:
            chain.append(frame_names[stack_frame[current]])
            current = stack_prefix[current]
        label, frame, read = bucket(chain)
        if label in {
            "main",
            "std::rt::lang_start_internal",
            "large_corpus_profile::main",
            "std::sys::backtrace::__rust_begin_short_backtrace",
        }:
            continue
        counts[label] += weight
        examples.setdefault(label, (frame, read))

    print(f"total_samples={total}")
    print("| Rank | Hotspot | Attributed Exclusive | Representative Frame | Read |")
    print("|---:|---|---:|---|---|")
    rank = 1
    for label, count in counts.most_common():
        percent = count / total * 100 if total else 0
        if rank > args.limit or percent < args.min_percent:
            break
        frame, read = examples[label]
        print(f"| {rank} | {label} | {percent:.1f}% | `{frame}` | {read} |")
        rank += 1


if __name__ == "__main__":
    main()
