"""Exercise the npm release step with a local tarball and no publish credentials."""

import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/npm-publish.yml"


def publish_script() -> str:
    marker = "      - name: Publish package\n        run: |\n"
    source = WORKFLOW.read_text()
    assert source.count(marker) == 1
    block = source.split(marker, 1)[1]
    lines = []
    for line in block.splitlines():
        if line and not line.startswith("          "):
            break
        lines.append(line[10:] if line else "")
    assert lines
    return "\n".join(lines)


def test_publish_step_uses_local_tarball() -> None:
    npm = shutil.which("npm")
    assert npm is not None

    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        package_dir = root / "npm-package"
        package_dir.mkdir()
        tarball = package_dir / "shuck-wasm-0.0.0.tgz"
        package_json = json.dumps({"name": "shuck-wasm", "version": "0.0.0"}).encode()
        with tarfile.open(tarball, "w:gz") as archive:
            entry = tarfile.TarInfo("package/package.json")
            entry.size = len(package_json)
            archive.addfile(entry, io.BytesIO(package_json))

        bin_dir = root / "bin"
        bin_dir.mkdir()
        npm_stub = bin_dir / "npm"
        npm_stub.write_text(
            "#!/bin/bash\n"
            'test "$1" = publish || exit 2\n'
            'test "$3" = --access && test "$4" = public || exit 2\n'
            f'exec "{npm}" pack --dry-run "$2" --json\n'
        )
        npm_stub.chmod(0o755)
        env = os.environ.copy()
        env["PATH"] = f"{bin_dir}:{env['PATH']}"
        env["npm_config_cache"] = str(root / "npm-cache")
        env["GIT_SSH_COMMAND"] = "false"

        result = subprocess.run(
            ["bash", "-e", "-c", publish_script()],
            cwd=root,
            env=env,
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
        )
        assert result.returncode == 0, result.stderr
        assert json.loads(result.stdout)[0]["name"] == "shuck-wasm"


if __name__ == "__main__":
    test_publish_step_uses_local_tarball()
