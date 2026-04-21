#!/usr/bin/env python3

from __future__ import annotations

import sys
import unittest
import importlib.util
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

MODULE_PATH = REPO_ROOT / "scripts" / "check-release-security.py"
SPEC = importlib.util.spec_from_file_location("check_release_security", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC is not None and SPEC.loader is not None
SPEC.loader.exec_module(MODULE)
check = MODULE.check
fix = MODULE.fix


SAMPLE_WORKFLOW = """\
name: Release
permissions:
  contents: write

jobs:
  plan:
    runs-on: ubuntu-22.04
  build-global-artifacts:
    runs-on: ubuntu-22.04
    steps:
      - run: chmod +x ~/.cargo/bin/dist
      - id: cargo-dist
        shell: bash
        run: |
          dist build ${{ needs.plan.outputs.tag-flag }} --output-format=json "--artifacts=global" > dist-manifest.json
      - name: Build artifacts
        run: |
          dist build ${{ needs.plan.outputs.tag-flag }} --print=linkage --output-format=json ${{ matrix.dist_args }} > dist-manifest.json
  host:
    runs-on: ubuntu-22.04
    env:
      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
    steps:
      - id: host
        shell: bash
        run: |
          dist host ${{ needs.plan.outputs.tag-flag }} --steps=upload --steps=release --output-format=json > dist-manifest.json
      - name: Create GitHub Release
        env:
          PRERELEASE_FLAG: "${{ fromJson(steps.host.outputs.manifest).announcement_is_prerelease && '--prerelease' || '' }}"
          ANNOUNCEMENT_TITLE: "${{ fromJson(steps.host.outputs.manifest).announcement_title }}"
          ANNOUNCEMENT_BODY: "${{ fromJson(steps.host.outputs.manifest).announcement_github_body }}"
          RELEASE_COMMIT: "${{ github.sha }}"
        run: |
          gh release create "${{ needs.plan.outputs.tag }}" --target "$RELEASE_COMMIT" $PRERELEASE_FLAG --title "$ANNOUNCEMENT_TITLE" --notes-file "$RUNNER_TEMP/notes.txt" artifacts/*
  publish-homebrew-formula:
    needs:
      - plan
      - host
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@deadbeef
        with:
          token: ${{ secrets.HOMEBREW_TAP_TOKEN }}
"""


class CheckReleaseSecurityTests(unittest.TestCase):
    def test_check_reports_new_release_workflow_risks(self) -> None:
        issues = check(SAMPLE_WORKFLOW)

        self.assertIn("top-level permissions grants write access", issues)
        self.assertIn("plan job missing per-job permissions", issues)
        self.assertIn("host job missing per-job permissions", issues)
        self.assertIn("host job missing environment: release", issues)
        self.assertIn(
            "dist build uses direct template expansion in run block", issues
        )
        self.assertIn(
            "host dist command uses direct template expansion in run block", issues
        )
        self.assertIn(
            "release creation uses direct template expansion in run block", issues
        )
        self.assertIn("publish-homebrew-formula missing environment: release", issues)
        self.assertIn(
            "build-global-artifacts missing cargo-cyclonedx install", issues
        )
        self.assertIn(
            "release publishing no longer updates existing GitHub releases", issues
        )

    def test_fix_hardens_generated_release_workflow(self) -> None:
        fixed = fix(SAMPLE_WORKFLOW)

        self.assertIn("    permissions:\n      contents: write", fixed)
        self.assertIn("    environment: release", fixed)
        self.assertIn(
            "          dist build ${NEEDS_PLAN_OUTPUTS_TAG_FLAG} --output-format=json \"--artifacts=global\" > dist-manifest.json",
            fixed,
        )
        self.assertIn(
            "          dist host ${NEEDS_PLAN_OUTPUTS_TAG_FLAG} --steps=upload --steps=release --output-format=json > dist-manifest.json",
            fixed,
        )
        self.assertIn(
            "          NEEDS_PLAN_OUTPUTS_TAG_FLAG: ${{ needs.plan.outputs.tag-flag }}",
            fixed,
        )
        self.assertIn(
            "          NEEDS_PLAN_OUTPUTS_TAG: ${{ needs.plan.outputs.tag }}",
            fixed,
        )
        self.assertIn("      - name: Install cargo-cyclonedx", fixed)
        self.assertIn(
            '            gh release upload "${NEEDS_PLAN_OUTPUTS_TAG}" artifacts/* --clobber',
            fixed,
        )
        self.assertIn(
            '          gh release create "${NEEDS_PLAN_OUTPUTS_TAG}" --target "$RELEASE_COMMIT" $PRERELEASE_FLAG --title "$ANNOUNCEMENT_TITLE" --notes-file "$RUNNER_TEMP/notes.txt" artifacts/*',
            fixed,
        )
        self.assertEqual(check(fixed), [])


if __name__ == "__main__":
    unittest.main()
