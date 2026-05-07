from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


def bundled_binary_path() -> Path:
    binary_name = "shuck.exe" if os.name == "nt" else "shuck"
    return Path(__file__).resolve().parent / "bin" / binary_name


def main() -> int:
    binary_path = bundled_binary_path()
    if not binary_path.is_file():
        print(
            f"shuck-cli wheel is missing bundled executable: {binary_path}",
            file=sys.stderr,
        )
        return 1

    argv = [os.fspath(binary_path), *sys.argv[1:]]
    if os.name != "nt":
        os.execv(os.fspath(binary_path), argv)
        return 0

    completed = subprocess.run(argv, check=False)
    return completed.returncode
