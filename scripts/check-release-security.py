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


def get_job_section(lines, job_name):
    """Return (start_line, end_line) indices for a job section."""
    start = None
    for i, line in enumerate(lines):
        if re.match(rf"^  {re.escape(job_name)}:\s*$", line.rstrip()):
            start = i
        elif start is not None and re.match(r"^  \w", line) and i > start:
            return start, i
    if start is not None:
        return start, len(lines)
    return None, None


def check(content):
    """Return list of security issues found."""
    lines = content.splitlines()
    issues = []

    # Check 1: top-level permissions must not grant write
    for i, line in enumerate(lines):
        if line.rstrip() == "permissions:" and not line.startswith(" "):
            for j in range(i + 1, len(lines)):
                perm_line = lines[j]
                stripped = perm_line.strip()
                if not stripped or stripped.startswith("#"):
                    continue
                if not perm_line.startswith("  ") or perm_line.startswith("    "):
                    break
                if "write" in perm_line:
                    issues.append("top-level permissions grants write access")
                    break
            break

    # Check 2: plan job has per-job permissions
    start, end = get_job_section(lines, "plan")
    if start is not None:
        section = "\n".join(lines[start:end])
        if not re.search(r"^\s{4}permissions:", section, re.MULTILINE):
            issues.append("plan job missing per-job permissions")

    # Check 3: host job has per-job permissions and environment gate
    start, end = get_job_section(lines, "host")
    if start is not None:
        section = "\n".join(lines[start:end])
        if not re.search(r"^\s{4}permissions:", section, re.MULTILINE):
            issues.append("host job missing per-job permissions")
        if "environment: release" not in section:
            issues.append("host job missing environment: release")

    return issues


def fix(content):
    """Apply security hardening fixes and return the patched content."""
    lines = content.splitlines()

    # Fix 1: top-level permissions -> read-only
    for i, line in enumerate(lines):
        if line.rstrip() == "permissions:" and not line.startswith(" "):
            for j in range(i + 1, len(lines)):
                perm_line = lines[j]
                stripped = perm_line.strip()
                if not stripped or stripped.startswith("#"):
                    continue
                if not perm_line.startswith("  ") or perm_line.startswith("    "):
                    break
                if "write" in perm_line:
                    lines[j] = re.sub(r"write\b", "read", perm_line)
            break

    # Fix 2: plan job — insert per-job permissions after runs-on
    start, end = get_job_section(lines, "plan")
    if start is not None:
        section_text = "\n".join(lines[start:end])
        if not re.search(r"^\s{4}permissions:", section_text, re.MULTILINE):
            for i in range(start, end):
                if lines[i].strip().startswith("runs-on:"):
                    lines.insert(i + 1, "    permissions:")
                    lines.insert(i + 2, "      contents: write")
                    break

    # Fix 3: host job — insert per-job permissions + environment before env:
    start, end = get_job_section(lines, "host")
    if start is not None:
        section_text = "\n".join(lines[start:end])
        has_perms = bool(
            re.search(r"^\s{4}permissions:", section_text, re.MULTILINE)
        )
        has_env = "environment: release" in section_text

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

    content = path.read_text()
    issues = check(content)

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
        remaining = check(fixed)
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
