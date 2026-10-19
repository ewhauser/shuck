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

TESTDATA = REPO_ROOT / "scripts" / "testdata"
RELEASE_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "release.yml"
CARGO_TOML = REPO_ROOT / "Cargo.toml"


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
      - uses: actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd
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

    def test_fix_hardens_freshly_generated_cargo_dist_workflow(self) -> None:
        generated = (TESTDATA / "cargo-dist-release.yml").read_text()
        self.assertNotEqual(check(generated), [])

        fixed = fix(generated)

        self.assertEqual(check(fixed), [])
        self.assertEqual(fix(fixed), fixed)
        self.assertIn(
            '            gh release edit "${NEEDS_PLAN_OUTPUTS_TAG}" --draft=false', fixed
        )
        self.assertIn("          GH_TOKEN: ${{ secrets.RELEASE_PAT }}", fixed)
        self.assertIn(
            "          dist ${DIST_PLAN_ARGS} --output-format=json > plan-dist-manifest.json",
            fixed,
        )
        self.assertIn(
            f"          DIST_INSTALLER_SHA256: {MODULE.DIST_INSTALLER_SHA256}", fixed
        )
        self.assertNotIn("cargo-dist-installer.sh | sh", fixed)
        self.assertNotIn("matrix.install_dist.run", fixed)

    def test_committed_release_workflow_passes(self) -> None:
        self.assertEqual(
            check(RELEASE_WORKFLOW.read_text(), CARGO_TOML.read_text()), []
        )

    def test_release_publishing_must_undraft_release_please_draft(self) -> None:
        workflow = RELEASE_WORKFLOW.read_text()
        without_edit = "\n".join(
            line
            for line in workflow.splitlines()
            if "--draft=false" not in line
        )

        self.assertIn(
            "release publishing no longer publishes the release-please draft",
            check(without_edit),
        )

    def test_top_level_permissions_must_exist_and_be_read_only(self) -> None:
        workflow = RELEASE_WORKFLOW.read_text()
        missing = workflow.replace('permissions:\n  "contents": "read"\n', "", 1)
        write_all = workflow.replace(
            'permissions:\n  "contents": "read"\n', "permissions: write-all\n", 1
        )

        self.assertIn("top-level permissions missing", check(missing))
        self.assertIn("top-level permissions grants write access", check(write_all))
        self.assertEqual(check(fix(missing)), [])
        self.assertEqual(check(fix(write_all)), [])

    def test_missing_hardened_jobs_fail_instead_of_skipping(self) -> None:
        workflow = RELEASE_WORKFLOW.read_text()
        renamed = workflow.replace("\n  host:\n", "\n  release-host:\n").replace(
            "\n  publish-homebrew-formula:\n", "\n  publish-brew:\n"
        )

        issues = check(renamed)

        self.assertIn("host job missing", issues)
        self.assertIn("publish-homebrew-formula job missing", issues)

    def test_environment_in_comment_does_not_count(self) -> None:
        workflow = RELEASE_WORKFLOW.read_text()
        commented = workflow.replace(
            "    environment: release\n    env:\n      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n    runs-on",
            "    # environment: release\n    env:\n      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n    runs-on",
            1,
        )
        mapping = workflow.replace(
            "    environment: release\n    env:\n      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n    runs-on",
            "    environment:\n      name: release\n    env:\n      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n    runs-on",
            1,
        )

        self.assertNotEqual(commented, workflow)
        self.assertIn("host job missing environment: release", check(commented))
        self.assertNotIn("host job missing environment: release", check(mapping))

    def test_ref_name_interpolated_into_run_is_reported(self) -> None:
        workflow = RELEASE_WORKFLOW.read_text().replace(
            "          dist ${DIST_PLAN_ARGS} --output-format=json",
            "          dist host --tag=${{ github.ref_name }} --output-format=json",
        )

        issues = check(workflow)

        self.assertTrue(
            any("interpolates ${{ github.ref_name }}" in issue for issue in issues),
            issues,
        )

    def test_unpinned_actions_are_reported(self) -> None:
        workflow = RELEASE_WORKFLOW.read_text().replace(
            "actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6",
            "actions/attest@v4",
        )

        issues = check(workflow)

        self.assertTrue(
            any("actions/attest@v4" in issue and "full commit SHA" in issue for issue in issues),
            issues,
        )

    def test_checkout_must_not_persist_credentials_outside_homebrew_job(self) -> None:
        workflow = RELEASE_WORKFLOW.read_text()
        persisted = workflow.replace(
            "          persist-credentials: false\n", "", 1
        )

        issues = check(persisted)

        self.assertTrue(
            any("in job plan does not set persist-credentials: false" in issue for issue in issues),
            issues,
        )
        self.assertEqual(check(fix(persisted)), [])
        # The Homebrew job pushes with its checkout token, so it is exempt.
        self.assertIn("persist-credentials: true", workflow)

    def test_unverified_dist_installer_is_reported(self) -> None:
        workflow = RELEASE_WORKFLOW.read_text()
        start = workflow.index("      - name: Install dist\n        shell: bash\n        env:")
        end = workflow.index("      # Get the dist-manifest")
        piped = (
            workflow[:start]
            + "      - name: Install dist\n        run: ${{ matrix.install_dist.run }}\n"
            + workflow[end:]
        )

        issues = check(piped)

        self.assertIn(
            "dist installer is piped to a shell without checksum verification", issues
        )
        self.assertEqual(check(fix(piped)), [])

    def test_cargo_dist_version_must_match_verified_installer(self) -> None:
        cargo_toml = CARGO_TOML.read_text().replace(
            'cargo-dist-version = "0.31.0"', 'cargo-dist-version = "0.99.0"'
        )

        issues = check(RELEASE_WORKFLOW.read_text(), cargo_toml)

        self.assertTrue(
            any("pins cargo-dist 0.99.0" in issue for issue in issues), issues
        )


if __name__ == "__main__":
    unittest.main()
