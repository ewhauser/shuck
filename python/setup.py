from __future__ import annotations

import os
from pathlib import Path

from setuptools import setup
from setuptools.command.bdist_wheel import bdist_wheel as _bdist_wheel
from setuptools.command.build_py import build_py as _build_py
from setuptools.dist import Distribution


PACKAGE_ROOT = Path(__file__).resolve().parent / "src" / "shuck_cli"
BIN_DIR = PACKAGE_ROOT / "bin"


class BinaryDistribution(Distribution):
    def has_ext_modules(self) -> bool:
        return True


class build_py(_build_py):
    def run(self) -> None:
        staged = [BIN_DIR / "shuck", BIN_DIR / "shuck.exe"]
        if not any(path.is_file() for path in staged):
            raise RuntimeError(
                "no staged shuck binary found under src/shuck_cli/bin; "
                "use scripts/build-python-release.py to prepare the wheel"
            )
        super().run()


class bdist_wheel(_bdist_wheel):
    def finalize_options(self) -> None:
        super().finalize_options()
        self.root_is_pure = False

        plat_name = os.environ.get("SHUCK_PYTHON_WHEEL_PLAT_NAME")
        if plat_name:
            self.plat_name_supplied = True
            self.plat_name = plat_name

    def get_tag(self) -> tuple[str, str, str]:
        python_tag = os.environ.get("SHUCK_PYTHON_WHEEL_PYTHON_TAG")
        abi_tag = os.environ.get("SHUCK_PYTHON_WHEEL_ABI_TAG")
        plat_name = os.environ.get("SHUCK_PYTHON_WHEEL_PLAT_NAME")
        if python_tag and abi_tag and plat_name:
            normalized_plat = plat_name.replace("-", "_").replace(".", "_")
            return python_tag, abi_tag, normalized_plat
        return super().get_tag()


setup(
    distclass=BinaryDistribution,
    cmdclass={
        "bdist_wheel": bdist_wheel,
        "build_py": build_py,
    },
)
