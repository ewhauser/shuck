#!/usr/bin/env python3
"""Build platform-specific shuck-cli wheels from staged binaries or release assets."""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from dataclasses import dataclass
from pathlib import Path


PYTHON_TAG = "py3"
ABI_TAG = "none"


@dataclass(frozen=True)
class TargetSpec:
    target: str
    archive_name: str
    packaged_binary_name: str
    wheel_platform_tag: str


TARGETS: dict[str, TargetSpec] = {
    "aarch64-apple-darwin": TargetSpec(
        target="aarch64-apple-darwin",
        archive_name="shuck-cli-aarch64-apple-darwin.tar.xz",
        packaged_binary_name="shuck",
        wheel_platform_tag="macosx_11_0_arm64",
    ),
    "aarch64-unknown-linux-gnu": TargetSpec(
        target="aarch64-unknown-linux-gnu",
        archive_name="shuck-cli-aarch64-unknown-linux-gnu.tar.xz",
        packaged_binary_name="shuck",
        wheel_platform_tag="manylinux_2_28_aarch64",
    ),
    "aarch64-unknown-linux-musl": TargetSpec(
        target="aarch64-unknown-linux-musl",
        archive_name="shuck-cli-aarch64-unknown-linux-musl.tar.xz",
        packaged_binary_name="shuck",
        wheel_platform_tag="musllinux_1_2_aarch64",
    ),
    "x86_64-pc-windows-msvc": TargetSpec(
        target="x86_64-pc-windows-msvc",
        archive_name="shuck-cli-x86_64-pc-windows-msvc.zip",
        packaged_binary_name="shuck.exe",
        wheel_platform_tag="win_amd64",
    ),
    "x86_64-unknown-linux-gnu": TargetSpec(
        target="x86_64-unknown-linux-gnu",
        archive_name="shuck-cli-x86_64-unknown-linux-gnu.tar.xz",
        packaged_binary_name="shuck",
        wheel_platform_tag="manylinux_2_28_x86_64",
    ),
    "x86_64-unknown-linux-musl": TargetSpec(
        target="x86_64-unknown-linux-musl",
        archive_name="shuck-cli-x86_64-unknown-linux-musl.tar.xz",
        packaged_binary_name="shuck",
        wheel_platform_tag="musllinux_1_2_x86_64",
    ),
}


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    build_wheel = subparsers.add_parser(
        "build-wheel",
        help="Build one wheel from a local binary or a downloaded release archive",
    )
    build_wheel.add_argument("--target", required=True, choices=sorted(TARGETS))
    build_wheel.add_argument("--out-dir", required=True)
    source_group = build_wheel.add_mutually_exclusive_group(required=True)
    source_group.add_argument("--archive", help="Path to a downloaded release archive")
    source_group.add_argument("--binary", help="Path to a compiled shuck executable")

    build_release = subparsers.add_parser(
        "build-release-wheels",
        help="Build wheels for every supported target from a directory of release assets",
    )
    build_release.add_argument("--assets-dir", required=True)
    build_release.add_argument("--out-dir", required=True)

    return parser.parse_args()


def stage_binary(source_binary: Path, spec: TargetSpec, build_root: Path) -> None:
    package_bin_dir = build_root / "src" / "shuck_cli" / "bin"
    package_bin_dir.mkdir(parents=True, exist_ok=True)

    destination = package_bin_dir / spec.packaged_binary_name
    shutil.copy2(source_binary, destination)
    if spec.packaged_binary_name == "shuck":
        destination.chmod(0o755)


def find_archive_binary(extracted_root: Path, spec: TargetSpec) -> Path:
    matches = sorted(extracted_root.rglob(spec.packaged_binary_name))
    if len(matches) != 1:
        raise SystemExit(
            f"expected exactly one {spec.packaged_binary_name} inside {extracted_root}, "
            f"found {len(matches)}"
        )
    return matches[0]


def extract_archive(archive_path: Path, spec: TargetSpec, tempdir: Path) -> Path:
    extracted_root = tempdir / "extracted"
    extracted_root.mkdir(parents=True, exist_ok=True)

    if archive_path.suffix == ".zip":
        with zipfile.ZipFile(archive_path) as archive:
            archive.extractall(extracted_root)
    else:
        with tarfile.open(archive_path, mode="r:*") as archive:
            archive.extractall(extracted_root)

    return find_archive_binary(extracted_root, spec)


def prepare_build_tree(spec: TargetSpec, source_binary: Path) -> Path:
    source_root = repo_root() / "python"
    tempdir = Path(tempfile.mkdtemp(prefix="shuck-python-wheel-"))
    build_root = tempdir / "python"
    shutil.copytree(source_root, build_root)
    stage_binary(source_binary, spec, build_root)
    return build_root


def build_wheel(build_root: Path, spec: TargetSpec, out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(
        {
            "SHUCK_PYTHON_WHEEL_ABI_TAG": ABI_TAG,
            "SHUCK_PYTHON_WHEEL_PLAT_NAME": spec.wheel_platform_tag,
            "SHUCK_PYTHON_WHEEL_PYTHON_TAG": PYTHON_TAG,
        }
    )
    subprocess.run(
        [
            sys.executable,
            "-m",
            "build",
            "--wheel",
            "--no-isolation",
            "--outdir",
            os.fspath(out_dir),
            os.fspath(build_root),
        ],
        check=True,
        env=env,
    )


def build_one_wheel(
    spec: TargetSpec,
    out_dir: Path,
    archive: str | None,
    binary: str | None,
) -> None:
    if archive is None and binary is None:
        raise SystemExit("expected either --archive or --binary")

    if archive is not None:
        archive_path = Path(archive).resolve()
        if not archive_path.is_file():
            raise SystemExit(f"release archive not found: {archive_path}")
        with tempfile.TemporaryDirectory(prefix="shuck-python-archive-") as tempdir:
            source_binary = extract_archive(archive_path, spec, Path(tempdir))
            build_root = prepare_build_tree(spec, source_binary)
            try:
                build_wheel(build_root, spec, out_dir)
            finally:
                shutil.rmtree(build_root.parent)
        return

    binary_path = Path(binary).resolve()
    if not binary_path.is_file():
        raise SystemExit(f"binary not found: {binary_path}")
    build_root = prepare_build_tree(spec, binary_path)
    try:
        build_wheel(build_root, spec, out_dir)
    finally:
        shutil.rmtree(build_root.parent)


def build_release_wheels(assets_dir: Path, out_dir: Path) -> None:
    missing = [
        spec.archive_name
        for spec in TARGETS.values()
        if not (assets_dir / spec.archive_name).is_file()
    ]
    if missing:
        joined = "\n".join(f"  - {name}" for name in missing)
        raise SystemExit(f"missing expected release assets:\n{joined}")

    for target in sorted(TARGETS):
        spec = TARGETS[target]
        build_one_wheel(
            spec,
            out_dir,
            archive=os.fspath(assets_dir / spec.archive_name),
            binary=None,
        )


def main() -> int:
    args = parse_args()

    if args.command == "build-wheel":
        spec = TARGETS[args.target]
        build_one_wheel(
            spec,
            Path(args.out_dir).resolve(),
            archive=args.archive,
            binary=args.binary,
        )
        return 0

    if args.command == "build-release-wheels":
        build_release_wheels(
            Path(args.assets_dir).resolve(),
            Path(args.out_dir).resolve(),
        )
        return 0

    raise AssertionError(f"unhandled command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
