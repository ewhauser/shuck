#!/usr/bin/env python3
"""Check and fix security hardening in the cargo-dist generated release workflow.

cargo-dist overwrites .github/workflows/release.yml on regeneration, removing
manual security hardening. Run after `cargo dist generate` to re-apply:

    python3 scripts/check-release-security.py fix

CI runs the check mode to catch regressions:

    python3 scripts/check-release-security.py check
"""

import re
import sys
from pathlib import Path

WORKFLOW_PATH = ".github/workflows/release.yml"
CARGO_TOML_PATH = "Cargo.toml"

# The cargo-dist installer is downloaded and verified against this digest before it
# runs. The installer embeds checksums for the dist archives it fetches, so pinning
# the installer pins the whole chain. Update both values together with
# `cargo-dist-version` in Cargo.toml.
DIST_VERSION = "v0.31.0"
DIST_INSTALLER_SHA256 = (
    "e79d87e418b9d2cbe992d014985457c28a5a7c553add3da4ed1047e161c928f4"
)

# Jobs the hardening below depends on. A regenerated workflow that drops or renames
# one of them must fail the check rather than silently skip it.
REQUIRED_JOBS = ("plan", "host", "publish-homebrew-formula")

# Jobs that must keep checkout credentials because they push with them.
CREDENTIAL_PERSISTING_JOBS = frozenset({"publish-homebrew-formula"})

# Expression contexts that carry ref names, event payloads, or job/step outputs and
# therefore must reach shell code through `env:` rather than inline interpolation.
EXPRESSION = re.compile(r"\$\{\{.*?\}\}")
UNTRUSTED_CONTEXT = re.compile(
    r"github\.(?:ref_name|head_ref|ref)\b|github\.event\.|\bneeds\.|\binputs\.|\bsteps\."
)

FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
RUN_KEY = re.compile(r"^(\s*)(-\s+)?run:\s*(.*)$")
USES_KEY = re.compile(r"""^(\s*)(-\s+)?uses:\s*["']?([^\s"'#]+)""")
ENVIRONMENT_RELEASE = re.compile(
    r"""^ {4}environment:\s*(["']?)release\1\s*(?:#.*)?$"""
)
ENVIRONMENT_MAPPING = re.compile(r"^ {4}environment:\s*(?:#.*)?$")
ENVIRONMENT_NAME_RELEASE = re.compile(r"""^ {6}name:\s*(["']?)release\1\s*(?:#.*)?$""")

RELEASE_TAG = '"${NEEDS_PLAN_OUTPUTS_TAG}"'
RELEASE_UPLOAD = f"gh release upload {RELEASE_TAG} artifacts/* --clobber"
RELEASE_PUBLISH_DRAFT = f"gh release edit {RELEASE_TAG} --draft=false"
RELEASE_PAT_ENV = "GH_TOKEN: ${{ secrets.RELEASE_PAT }}"

PLAN_EXPRESSION = (
    "${{ (!github.event.pull_request && "
    "format('host --steps=create --tag={0}', github.ref_name)) || 'plan' }}"
)


def indent_of(line):
    return len(line) - len(line.lstrip(" "))


def is_blank_or_comment(line):
    stripped = line.strip()
    return not stripped or stripped.startswith("#")


def get_job_section(lines, job_name):
    """Return (start_line, end_line) indices for a job section."""
    start = None
    for i, line in enumerate(lines):
        if re.match(rf"^  {re.escape(job_name)}:\s*(?:#.*)?$", line.rstrip()):
            start = i
        elif start is not None and re.match(r"^  \S", line) and not line.startswith(
            "  #"
        ):
            return start, i
        elif start is not None and re.match(r"^\S", line):
            return start, i
    if start is not None:
        return start, len(lines)
    return None, None


def job_for_line(lines, index):
    """Return the name of the job containing line `index`, if any."""
    for i in range(index, -1, -1):
        match = re.match(r"^  ([\w-]+):\s*(?:#.*)?$", lines[i])
        if match:
            return match.group(1)
        if re.match(r"^\S", lines[i]):
            return None
    return None


def has_job_permissions(lines, start, end):
    return any(re.match(r"^ {4}permissions:", line) for line in lines[start:end])


def has_release_environment(lines, start, end):
    """Whether the job declares `environment: release` (inline or mapping form)."""
    for i in range(start, end):
        if ENVIRONMENT_RELEASE.match(lines[i]):
            return True
        if ENVIRONMENT_MAPPING.match(lines[i]):
            for j in range(i + 1, end):
                if is_blank_or_comment(lines[j]):
                    continue
                if indent_of(lines[j]) <= 4:
                    break
                if ENVIRONMENT_NAME_RELEASE.match(lines[j]):
                    return True
    return False


def top_level_permissions_issue(lines):
    for i, line in enumerate(lines):
        match = re.match(r"^permissions:\s*(.*?)\s*(?:#.*)?$", line)
        if not match:
            continue
        inline = match.group(1).strip().strip("\"'")
        if inline:
            if inline == "write-all" or "write" in inline:
                return "top-level permissions grants write access"
            return None
        for j in range(i + 1, len(lines)):
            perm_line = lines[j]
            if is_blank_or_comment(perm_line):
                continue
            if not perm_line.startswith("  ") or perm_line.startswith("    "):
                break
            if "write" in perm_line.split("#", 1)[0]:
                return "top-level permissions grants write access"
        return None
    return "top-level permissions missing"


def step_bounds(lines, index):
    """Return (start, end) of the workflow step containing line `index`."""
    line = lines[index]
    match = re.match(r"^(\s*)-\s", line)
    if match:
        start = index
        dash_col = len(match.group(1))
    else:
        key_col = indent_of(line)
        dash_col = key_col - 2
        start = None
        for i in range(index - 1, -1, -1):
            if re.match(rf"^ {{{dash_col}}}-\s", lines[i]):
                start = i
                break
            if not is_blank_or_comment(lines[i]) and indent_of(lines[i]) < key_col:
                break
        if start is None:
            return index, index + 1
    end = len(lines)
    for i in range(start + 1, len(lines)):
        if is_blank_or_comment(lines[i]):
            continue
        if indent_of(lines[i]) <= dash_col:
            end = i
            break
    return start, end


def run_blocks(lines):
    """Yield (line_index, text) for every `run:` script in the workflow."""
    for i, line in enumerate(lines):
        match = RUN_KEY.match(line)
        if not match:
            continue
        key_col = line.index("run:")
        value = match.group(3)
        if value.startswith(("|", ">")):
            body = []
            for j in range(i + 1, len(lines)):
                if lines[j].strip() and indent_of(lines[j]) <= key_col:
                    break
                body.append(lines[j])
            yield i, "\n".join(body)
        else:
            yield i, value


def uses_refs(lines):
    """Yield (line_index, action_reference) for every `uses:` entry."""
    for i, line in enumerate(lines):
        match = USES_KEY.match(line)
        if match:
            yield i, match.group(3)


def install_dist_steps(lines):
    """Yield (start, end) for each step that installs dist from the network."""
    for i, line in enumerate(lines):
        if re.match(r"^\s*(-\s+)?name:\s*[\"']?Install dist[\"']?\s*$", line):
            yield step_bounds(lines, i)


def check(content, cargo_toml=None):
    """Return list of security issues found."""
    lines = content.splitlines()
    issues = []

    # Check 1: top-level permissions must exist and must not grant write.
    issue = top_level_permissions_issue(lines)
    if issue:
        issues.append(issue)

    # Check 2: the jobs we harden must exist.
    for job in REQUIRED_JOBS:
        if get_job_section(lines, job)[0] is None:
            issues.append(f"{job} job missing")

    # Check 3: plan job has per-job permissions
    start, end = get_job_section(lines, "plan")
    if start is not None and not has_job_permissions(lines, start, end):
        issues.append("plan job missing per-job permissions")

    # Check 4: host job has per-job permissions and environment gate
    start, end = get_job_section(lines, "host")
    if start is not None:
        if not has_job_permissions(lines, start, end):
            issues.append("host job missing per-job permissions")
        if not has_release_environment(lines, start, end):
            issues.append("host job missing environment: release")

    # Check 5: release commands should not interpolate needs.* expressions directly in shell.
    if "dist build ${{ needs.plan.outputs.tag-flag }}" in content:
        issues.append("dist build uses direct template expansion in run block")
    if "dist host ${{ needs.plan.outputs.tag-flag }}" in content:
        issues.append("host dist command uses direct template expansion in run block")
    if 'gh release create "${{ needs.plan.outputs.tag }}"' in content:
        issues.append("release creation uses direct template expansion in run block")
    for index, body in run_blocks(lines):
        for expression in EXPRESSION.findall(body):
            if not UNTRUSTED_CONTEXT.search(expression):
                continue
            issues.append(
                f"run block at line {index + 1} interpolates {expression}; pass it through env:"
            )

    # Check 6: Homebrew publishing uses a secret and must be behind the release environment.
    start, end = get_job_section(lines, "publish-homebrew-formula")
    if start is not None and not has_release_environment(lines, start, end):
        issues.append("publish-homebrew-formula missing environment: release")

    # Check 7: global release artifacts still need cargo-cyclonedx for the SBOM extra artifact.
    start, end = get_job_section(lines, "build-global-artifacts")
    if start is not None:
        section = "\n".join(lines[start:end])
        if "Install cargo-cyclonedx" not in section:
            issues.append("build-global-artifacts missing cargo-cyclonedx install")

    # Check 8: release-please creates a draft release for the tag. Publishing must
    # upload into it and then un-draft it with a token whose events cascade.
    if RELEASE_UPLOAD not in content:
        issues.append("release publishing no longer updates existing GitHub releases")
    if RELEASE_PUBLISH_DRAFT not in content:
        issues.append("release publishing no longer publishes the release-please draft")
    start, end = get_job_section(lines, "host")
    if start is not None and RELEASE_PAT_ENV not in "\n".join(lines[start:end]):
        issues.append("release publishing does not use RELEASE_PAT")

    # Check 9: every action must be pinned to a full commit SHA.
    for index, reference in uses_refs(lines):
        if reference.startswith(("./", "docker://")):
            continue
        _, _, ref = reference.partition("@")
        if not FULL_SHA.match(ref):
            issues.append(
                f"action {reference} at line {index + 1} is not pinned to a full commit SHA"
            )

    # Check 10: checkouts must not leave the token in .git/config, except in jobs
    # that push with it.
    for index, reference in uses_refs(lines):
        if not reference.startswith("actions/checkout@"):
            continue
        job = job_for_line(lines, index)
        if job in CREDENTIAL_PERSISTING_JOBS:
            continue
        start, end = step_bounds(lines, index)
        step = "\n".join(lines[start:end])
        if not re.search(r"persist-credentials:\s*false\b", step):
            issues.append(
                f"checkout at line {index + 1} in job {job} does not set persist-credentials: false"
            )

    # Check 11: the dist installer must be verified before it runs.
    if "matrix.install_dist.run" in content or re.search(
        r"cargo-dist-installer\.(?:sh|ps1)[^\n]*\|\s*(?:sh|bash|iex)\b", content
    ):
        issues.append("dist installer is piped to a shell without checksum verification")
    for start, end in install_dist_steps(lines):
        step = "\n".join(lines[start:end])
        if (
            f"DIST_INSTALLER_SHA256: {DIST_INSTALLER_SHA256}" not in step
            or f"DIST_VERSION: {DIST_VERSION}" not in step
            or "--check" not in step
        ):
            issues.append(
                f"Install dist step at line {start + 1} does not verify the pinned installer checksum"
            )
    if cargo_toml is not None:
        match = re.search(r'^cargo-dist-version\s*=\s*"([^"]+)"', cargo_toml, re.MULTILINE)
        if match and f"v{match.group(1)}" != DIST_VERSION:
            issues.append(
                f"Cargo.toml pins cargo-dist {match.group(1)} but the verified installer is "
                f"{DIST_VERSION}; update DIST_VERSION and DIST_INSTALLER_SHA256"
            )

    return issues


def verified_dist_install_lines(step_indent):
    """Return `env:` + `run:` lines that download, verify, and run the dist installer."""
    key = " " * step_indent
    body = " " * (step_indent + 2)
    return [
        f"{key}env:",
        f"{body}DIST_VERSION: {DIST_VERSION}",
        f"{body}DIST_INSTALLER_SHA256: {DIST_INSTALLER_SHA256}",
        f"{key}run: |",
        f'{body}installer="$(mktemp -d)/cargo-dist-installer.sh"',
        f"{body}curl --proto '=https' --tlsv1.2 -LsSf "
        '"https://github.com/axodotdev/cargo-dist/releases/download/${DIST_VERSION}/cargo-dist-installer.sh" '
        '--output "$installer"',
        f"{body}if command -v sha256sum >/dev/null 2>&1; then",
        f'{body}  echo "${{DIST_INSTALLER_SHA256}}  ${{installer}}" | sha256sum --check --strict',
        f"{body}else",
        f'{body}  echo "${{DIST_INSTALLER_SHA256}}  ${{installer}}" | shasum -a 256 --check',
        f"{body}fi",
        f'{body}sh "$installer"',
    ]


def fix(content):
    """Apply security hardening fixes and return the patched content."""
    lines = content.splitlines()

    # Fix 1: top-level permissions -> read-only (and present).
    found_permissions = False
    for i, line in enumerate(lines):
        match = re.match(r"^permissions:\s*(.*?)\s*$", line)
        if not match:
            continue
        found_permissions = True
        if match.group(1) and "write" in match.group(1):
            lines[i : i + 1] = ["permissions:", '  "contents": "read"']
            break
        for j in range(i + 1, len(lines)):
            perm_line = lines[j]
            if is_blank_or_comment(perm_line):
                continue
            if not perm_line.startswith("  ") or perm_line.startswith("    "):
                break
            if "write" in perm_line:
                lines[j] = re.sub(r"write\b", "read", perm_line)
        break
    if not found_permissions:
        for i, line in enumerate(lines):
            if re.match(r"^name:", line):
                lines[i + 1 : i + 1] = ["permissions:", '  "contents": "read"']
                break

    # Fix 2: plan job — insert per-job permissions after runs-on
    start, end = get_job_section(lines, "plan")
    if start is not None and not has_job_permissions(lines, start, end):
        for i in range(start, end):
            if lines[i].strip().startswith("runs-on:"):
                lines.insert(i + 1, "    permissions:")
                lines.insert(i + 2, "      contents: write")
                break

    # Fix 3: host job — insert per-job permissions + environment before env:
    start, end = get_job_section(lines, "host")
    if start is not None:
        has_perms = has_job_permissions(lines, start, end)
        has_env = has_release_environment(lines, start, end)

        if not has_perms or not has_env:
            for i in range(start, end):
                if lines[i].strip().startswith("env:") and lines[i].startswith(
                    "    "
                ):
                    insert = []
                    if not has_perms:
                        insert.extend(["    permissions:", "      contents: write"])
                    if not has_env:
                        insert.append("    environment: release")
                    for j, new_line in enumerate(insert):
                        lines.insert(i + j, new_line)
                    break

    # Fix 4: move generated expressions out of run blocks and into env vars.
    substitutions = {
        "          dist build ${{ needs.plan.outputs.tag-flag }} --print=linkage --output-format=json ${{ matrix.dist_args }} > dist-manifest.json":
        "          dist build ${NEEDS_PLAN_OUTPUTS_TAG_FLAG} --print=linkage --output-format=json ${{ matrix.dist_args }} > dist-manifest.json",
        "          dist build ${{ needs.plan.outputs.tag-flag }} --output-format=json \"--artifacts=global\" > dist-manifest.json":
        "          dist build ${NEEDS_PLAN_OUTPUTS_TAG_FLAG} --output-format=json \"--artifacts=global\" > dist-manifest.json",
        "          dist host ${{ needs.plan.outputs.tag-flag }} --steps=upload --steps=release --output-format=json > dist-manifest.json":
        "          dist host ${NEEDS_PLAN_OUTPUTS_TAG_FLAG} --steps=upload --steps=release --output-format=json > dist-manifest.json",
        "          gh release create \"${{ needs.plan.outputs.tag }}\" --target \"$RELEASE_COMMIT\" $PRERELEASE_FLAG --title \"$ANNOUNCEMENT_TITLE\" --notes-file \"$RUNNER_TEMP/notes.txt\" artifacts/*":
        "          gh release create \"${NEEDS_PLAN_OUTPUTS_TAG}\" --target \"$RELEASE_COMMIT\" $PRERELEASE_FLAG --title \"$ANNOUNCEMENT_TITLE\" --notes-file \"$RUNNER_TEMP/notes.txt\" artifacts/*",
        f"          dist {PLAN_EXPRESSION} --output-format=json > plan-dist-manifest.json":
        # Tag names cannot contain whitespace or glob characters, so word splitting
        # here only separates the generated subcommand arguments.
        "          dist ${DIST_PLAN_ARGS} --output-format=json > plan-dist-manifest.json",
    }
    lines = [substitutions.get(line, line) for line in lines]

    for i, line in enumerate(lines):
        if line == "      - name: Build artifacts" and i + 1 < len(lines):
            if lines[i + 1] == "        run: |":
                lines.insert(i + 1, "        env:")
                lines.insert(
                    i + 2,
                    "          NEEDS_PLAN_OUTPUTS_TAG_FLAG: ${{ needs.plan.outputs.tag-flag }}",
                )
            break

    for i, line in enumerate(lines):
        if line == "      - id: cargo-dist" and i + 2 < len(lines):
            if (
                lines[i + 1] == "        shell: bash"
                and lines[i + 2] == "        run: |"
            ):
                lines.insert(i + 2, "        env:")
                lines.insert(
                    i + 3,
                    "          NEEDS_PLAN_OUTPUTS_TAG_FLAG: ${{ needs.plan.outputs.tag-flag }}",
                )
            break

    for i, line in enumerate(lines):
        if line == "      - id: host" and i + 2 < len(lines):
            if (
                lines[i + 1] == "        shell: bash"
                and lines[i + 2] == "        run: |"
            ):
                lines.insert(i + 2, "        env:")
                lines.insert(
                    i + 3,
                    "          NEEDS_PLAN_OUTPUTS_TAG_FLAG: ${{ needs.plan.outputs.tag-flag }}",
                )
            break

    for i, line in enumerate(lines):
        if line == "      - id: plan" and i + 1 < len(lines):
            if lines[i + 1] == "        run: |":
                lines.insert(i + 1, "        env:")
                lines.insert(i + 2, f"          DIST_PLAN_ARGS: {PLAN_EXPRESSION}")
            break

    for i, line in enumerate(lines):
        if line == '          RELEASE_COMMIT: "${{ github.sha }}"':
            if (
                i + 1 < len(lines)
                and lines[i + 1]
                != "          NEEDS_PLAN_OUTPUTS_TAG: ${{ needs.plan.outputs.tag }}"
            ):
                lines.insert(
                    i + 1,
                    "          NEEDS_PLAN_OUTPUTS_TAG: ${{ needs.plan.outputs.tag }}",
                )
            break

    # Fix 5: require the protected release environment for Homebrew publish job.
    start, end = get_job_section(lines, "publish-homebrew-formula")
    if start is not None and not has_release_environment(lines, start, end):
        for i in range(start, end):
            if lines[i].strip().startswith("runs-on:"):
                lines.insert(i + 1, "    environment: release")
                break

    # Fix 6: restore cargo-cyclonedx for the release SBOM artifact.
    start, end = get_job_section(lines, "build-global-artifacts")
    if start is not None:
        section_text = "\n".join(lines[start:end])
        if "Install cargo-cyclonedx" not in section_text:
            for i in range(start, end):
                if lines[i] == "      - run: chmod +x ~/.cargo/bin/dist":
                    lines.insert(i + 1, "      - name: Install cargo-cyclonedx")
                    lines.insert(i + 2, "        run: |")
                    lines.insert(
                        i + 3,
                        "          if ! cargo cyclonedx --version >/dev/null 2>&1; then",
                    )
                    lines.insert(
                        i + 4,
                        "            cargo install cargo-cyclonedx --locked --version 0.5.9",
                    )
                    lines.insert(i + 5, "          fi")
                    break

    # Fix 7: upload into the release-please draft and publish it, falling back to
    # creating the release when no draft exists yet.
    simple_release = (
        '          gh release create "${NEEDS_PLAN_OUTPUTS_TAG}" --target "$RELEASE_COMMIT" '
        '$PRERELEASE_FLAG --title "$ANNOUNCEMENT_TITLE" --notes-file "$RUNNER_TEMP/notes.txt" artifacts/*'
    )
    for i, line in enumerate(lines):
        if line == simple_release:
            lines[i : i + 1] = [
                f"          if gh release view {RELEASE_TAG} >/dev/null 2>&1; then",
                f"            {RELEASE_UPLOAD}",
                f"            {RELEASE_PUBLISH_DRAFT}",
                "          else",
                '            gh release create "${NEEDS_PLAN_OUTPUTS_TAG}" --target "$RELEASE_COMMIT" $PRERELEASE_FLAG --title "$ANNOUNCEMENT_TITLE" --notes-file "$RUNNER_TEMP/notes.txt" artifacts/*',
                "          fi",
            ]
            break

    start, end = get_job_section(lines, "host")
    if start is not None and RELEASE_PAT_ENV not in "\n".join(lines[start:end]):
        for i in range(start, end - 1):
            if re.match(
                r"^      - name: [\"']?Create (?:or update )?GitHub Release", lines[i]
            ) and lines[i + 1] == "        env:":
                lines[i + 2 : i + 2] = [
                    "          # Use RELEASE_PAT (not GITHUB_TOKEN) so `gh release edit --draft=false`",
                    "          # triggers the `release: published` event in release-please.yml.",
                    "          # GITHUB_TOKEN-driven events don't cascade to other workflows.",
                    f"          {RELEASE_PAT_ENV}",
                ]
                break

    # Fix 8: download the dist installer, verify it, then run it.
    for i, line in enumerate(lines):
        if re.match(
            r"""^\s*run:\s*["']?curl [^\n]*cargo-dist-installer\.sh \| sh["']?\s*$""",
            line,
        ) or re.match(r"^\s*run:\s*\$\{\{\s*matrix\.install_dist\.run\s*\}\}\s*$", line):
            step_indent = indent_of(line)
            start, _ = step_bounds(lines, i)
            replacement = verified_dist_install_lines(step_indent)
            step_text = "\n".join(lines[start:i])
            if not re.search(r"^\s*shell:\s*bash\s*$", step_text, re.MULTILINE):
                replacement.insert(0, f"{' ' * step_indent}shell: bash")
            lines[i : i + 1] = replacement

    # Fix 9: drop persisted checkout credentials where the job doesn't push.
    i = 0
    while i < len(lines):
        match = USES_KEY.match(lines[i])
        if (
            match
            and match.group(3).startswith("actions/checkout@")
            and job_for_line(lines, i) not in CREDENTIAL_PERSISTING_JOBS
        ):
            start, end = step_bounds(lines, i)
            step = "\n".join(lines[start:end])
            if not re.search(r"persist-credentials:\s*false\b", step):
                key_indent = indent_of(lines[i]) + (2 if match.group(2) else 0)
                key = " " * key_indent
                with_index = next(
                    (j for j in range(start, end) if lines[j] == f"{key}with:"), None
                )
                existing = next(
                    (
                        j
                        for j in range(start, end)
                        if re.match(r"^\s*persist-credentials:", lines[j])
                    ),
                    None,
                )
                if existing is not None:
                    lines[existing] = re.sub(
                        r"persist-credentials:.*$", "persist-credentials: false", lines[existing]
                    )
                elif with_index is not None:
                    lines.insert(with_index + 1, f"{key}  persist-credentials: false")
                else:
                    lines[end:end] = [f"{key}with:", f"{key}  persist-credentials: false"]
        i += 1

    return "\n".join(lines) + "\n"


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in ("check", "fix"):
        print(f"Usage: {sys.argv[0]} check|fix", file=sys.stderr)
        sys.exit(2)

    mode = sys.argv[1]
    path = Path(WORKFLOW_PATH)

    if not path.exists():
        print(f"{WORKFLOW_PATH} not found", file=sys.stderr)
        sys.exit(1)

    cargo_toml_path = Path(CARGO_TOML_PATH)
    cargo_toml = cargo_toml_path.read_text() if cargo_toml_path.exists() else None

    content = path.read_text()
    issues = check(content, cargo_toml)

    if mode == "check":
        if issues:
            print(f"{WORKFLOW_PATH}: {len(issues)} security issue(s):")
            for issue in issues:
                print(f"  - {issue}")
            sys.exit(1)
        print(f"{WORKFLOW_PATH}: all security checks passed")
    else:
        if not issues:
            print(f"{WORKFLOW_PATH}: nothing to fix")
            return
        fixed = fix(content)
        path.write_text(fixed)
        remaining = check(fixed, cargo_toml)
        if remaining:
            print(
                f"Could not auto-fix {len(remaining)} issue(s):", file=sys.stderr
            )
            for issue in remaining:
                print(f"  - {issue}", file=sys.stderr)
            sys.exit(1)
        print(f"{WORKFLOW_PATH}: fixed {len(issues)} issue(s)")
        for issue in issues:
            print(f"  - {issue}")


if __name__ == "__main__":
    main()
