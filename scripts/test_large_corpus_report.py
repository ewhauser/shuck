import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).with_name("large_corpus_report.py")
SPEC = importlib.util.spec_from_file_location("large_corpus_report", SCRIPT_PATH)
assert SPEC is not None
assert SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class LargeCorpusReportParsingTests(unittest.TestCase):
    def test_main_fixture_total_prefers_progress_total_over_summary_fixture_count(
        self,
    ) -> None:
        log = """large corpus: processed 3/6 fixtures (50%)
large corpus: processed 6/6 fixtures (100%)
large corpus compatibility summary: blocking=0 warnings=0 fixtures=10 unsupported_shells=4 implementation_diffs=0 mapping_issues=0 reviewed_divergences=0 harness_warnings=0 harness_failures=0
test large_corpus_conforms_with_shellcheck ... ok
"""

        self.assertEqual(MODULE.main_fixture_total(log, 10, 4), 6)

    def test_main_fixture_total_falls_back_to_supported_fixture_count_without_progress(
        self,
    ) -> None:
        log = """large corpus compatibility summary: blocking=0 warnings=0 fixtures=10 unsupported_shells=4 implementation_diffs=0 mapping_issues=0 reviewed_divergences=0 harness_warnings=0 harness_failures=0
test large_corpus_conforms_with_shellcheck ... ok
"""

        self.assertEqual(MODULE.main_fixture_total(log, 10, 4), 6)

    def test_extract_main_report_body_ignores_sections_after_main_result(self) -> None:
        log = """large corpus compatibility summary: blocking=0 warnings=0 fixtures=1 unsupported_shells=0 implementation_diffs=0 mapping_issues=0 reviewed_divergences=0 harness_warnings=0 harness_failures=0
test large_corpus_conforms_with_shellcheck ... ok
Harness Warnings:
/tmp/zsh-fixture.sh
  shuck timed out after 30.000s
"""

        self.assertIsNone(MODULE.extract_main_report_body(log))

    def test_extract_main_report_body_stays_within_main_test_span(self) -> None:
        log = """large corpus compatibility summary: blocking=0 warnings=1 fixtures=1 unsupported_shells=0 implementation_diffs=0 mapping_issues=1 reviewed_divergences=0 harness_warnings=0 harness_failures=0
Mapping Issues:
/tmp/main-fixture.sh
  shellcheck-only S032/SC2209 1:1-1:5 warning reason=main issue
test large_corpus_conforms_with_shellcheck ... ok
Harness Warnings:
/tmp/zsh-fixture.sh
  shuck timed out after 30.000s
"""

        body = MODULE.extract_main_report_body(log)

        self.assertIsNotNone(body)
        self.assertIn("Mapping Issues:", body)
        self.assertNotIn("/tmp/zsh-fixture.sh", body)

    def test_main_summary_keeps_mapping_and_reviewed_totals_record_based(self) -> None:
        log = """large corpus compatibility summary: blocking=0 warnings=2 fixtures=1 unsupported_shells=0 implementation_diffs=0 mapping_issues=1 reviewed_divergences=1 harness_warnings=0 harness_failures=0
Mapping Issues:
/tmp/main-fixture.sh
  shellcheck-only S032/SC2209 1:1-1:5 warning reason=first mapping
  shellcheck-only S032/SC2209 2:1-2:5 warning reason=second mapping

Reviewed Divergence:
/tmp/main-fixture.sh
  shuck-only C001/SC2034 3:1-3:5 warning reason=first reviewed
  shuck-only C001/SC2034 4:1-4:5 warning reason=second reviewed
test large_corpus_conforms_with_shellcheck ... ok
"""

        with tempfile.TemporaryDirectory() as tempdir:
            log_path = Path(tempdir) / "large-corpus.log"
            output_path = Path(tempdir) / "report.html"
            log_path.write_text(log, encoding="utf-8")

            subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT_PATH),
                    "--log",
                    str(log_path),
                    "--output",
                    str(output_path),
                ],
                check=True,
            )

            html = output_path.read_text(encoding="utf-8")

        self.assertIn("mapping issues, 2 reviewed divergences", html)

    def test_main_summary_falls_back_when_nonblocking_sections_are_omitted(self) -> None:
        log = """large corpus compatibility summary: blocking=1 warnings=13 fixtures=1 unsupported_shells=2 implementation_diffs=1 mapping_issues=3 reviewed_divergences=4 harness_warnings=6 harness_failures=0
Implementation Diffs:
/tmp/main-fixture.sh
  shellcheck-only C001/SC2000 1:1-1:5 error reason=blocking

Nonblocking issue buckets were omitted from the failing log output. See the compatibility summary counts above for skipped unsupported shells, mapping issues, reviewed divergences, and harness warnings.
test large_corpus_conforms_with_shellcheck ... FAILED
"""

        with tempfile.TemporaryDirectory() as tempdir:
            log_path = Path(tempdir) / "large-corpus.log"
            output_path = Path(tempdir) / "report.html"
            log_path.write_text(log, encoding="utf-8")

            subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT_PATH),
                    "--log",
                    str(log_path),
                    "--output",
                    str(output_path),
                ],
                check=True,
            )

            html = output_path.read_text(encoding="utf-8")

        self.assertIn("3\n        mapping issues, 4 reviewed divergences,", html)
        self.assertIn("6 main harness warnings,", html)

    def test_rendered_fixture_card_uses_progress_total(self) -> None:
        log = """large corpus: processed 3/6 fixtures (50%)
large corpus: processed 6/6 fixtures (100%)
large corpus compatibility summary: blocking=0 warnings=0 fixtures=10 unsupported_shells=4 implementation_diffs=0 mapping_issues=0 reviewed_divergences=0 harness_warnings=0 harness_failures=0
test large_corpus_conforms_with_shellcheck ... ok
"""

        with tempfile.TemporaryDirectory() as tempdir:
            log_path = Path(tempdir) / "large-corpus.log"
            output_path = Path(tempdir) / "report.html"
            log_path.write_text(log, encoding="utf-8")

            subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT_PATH),
                    "--log",
                    str(log_path),
                    "--output",
                    str(output_path),
                ],
                check=True,
            )

            html = output_path.read_text(encoding="utf-8")

        self.assertIn('<p class="value">6</p>', html)
        self.assertIn("largest observed progress count in the log", html)

    def test_reviewed_divergence_filter_keeps_only_known_failures(self) -> None:
        section = """/tmp/keep.sh
  shuck-only C001/SC2034 3:1-3:5 warning reason=known large-corpus rule allowlist

/tmp/drop.sh
  shuck-only C003/SC1091 4:1-4:5 warning reason=metadata-backed reviewed divergence
"""

        filtered = MODULE.filter_reviewed_divergence_section_for_known_failures(section)

        self.assertIsNotNone(filtered)
        self.assertIn("/tmp/keep.sh", filtered)
        self.assertNotIn("/tmp/drop.sh", filtered)

    def test_reviewed_divergence_only_run_still_populates_rule_and_fixture_tables(self) -> None:
        log = """large corpus compatibility summary: blocking=0 warnings=2 fixtures=1 unsupported_shells=0 implementation_diffs=0 mapping_issues=0 reviewed_divergences=2 harness_warnings=0 harness_failures=0
Reviewed Divergence:
/tmp/main-fixture.sh
  shuck-only C001/SC2034 3:1-3:5 warning reason=known large-corpus rule allowlist
  shuck-only C001/SC2034 4:1-4:5 warning reason=known large-corpus rule allowlist
test large_corpus_conforms_with_shellcheck ... ok
"""

        with tempfile.TemporaryDirectory() as tempdir:
            log_path = Path(tempdir) / "large-corpus.log"
            output_path = Path(tempdir) / "report.html"
            log_path.write_text(log, encoding="utf-8")

            subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT_PATH),
                    "--log",
                    str(log_path),
                    "--output",
                    str(output_path),
                ],
                check=True,
            )

            html = output_path.read_text(encoding="utf-8")

        self.assertIn("Top 5 rules account for 100.0% of all rule-coded records.", html)
        self.assertIn('<span class="badge">C001</span>', html)
        self.assertIn("Known Failure", html)
        self.assertIn("known large-corpus rule allowlist", html)

    def test_metadata_backed_reviewed_divergence_stays_out_of_detailed_tables(self) -> None:
        log = """large corpus compatibility summary: blocking=0 warnings=1 fixtures=1 unsupported_shells=0 implementation_diffs=0 mapping_issues=0 reviewed_divergences=1 harness_warnings=0 harness_failures=0
Reviewed Divergence:
/tmp/main-fixture.sh
  shuck-only C003/SC1091 3:1-3:5 warning reason=metadata-backed reviewed divergence
test large_corpus_conforms_with_shellcheck ... ok
"""

        with tempfile.TemporaryDirectory() as tempdir:
            log_path = Path(tempdir) / "large-corpus.log"
            output_path = Path(tempdir) / "report.html"
            log_path.write_text(log, encoding="utf-8")

            subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT_PATH),
                    "--log",
                    str(log_path),
                    "--output",
                    str(output_path),
                ],
                check=True,
            )

            html = output_path.read_text(encoding="utf-8")

        self.assertNotIn('<span class="badge">C003</span>', html)
        self.assertNotIn("metadata-backed reviewed divergence", html)
        self.assertIn("1 reviewed divergences", html)

    def test_main_timeout_cap_note_is_rendered(self) -> None:
        log = """large corpus compatibility summary: blocking=0 warnings=5 fixtures=1 unsupported_shells=0 implementation_diffs=0 mapping_issues=0 reviewed_divergences=0 harness_warnings=5 harness_failures=0
large corpus compatibility note: only the first 5 fixture timeouts were recorded as harness warnings; additional timeout fixtures were omitted.
Harness Warnings:
/tmp/main-fixture.sh
  shuck error: timed out after 30.000s
test large_corpus_conforms_with_shellcheck ... ok
"""

        with tempfile.TemporaryDirectory() as tempdir:
            log_path = Path(tempdir) / "large-corpus.log"
            output_path = Path(tempdir) / "report.html"
            log_path.write_text(log, encoding="utf-8")

            subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT_PATH),
                    "--log",
                    str(log_path),
                    "--output",
                    str(output_path),
                ],
                check=True,
            )

            html = output_path.read_text(encoding="utf-8")

        self.assertIn("only the first 5 fixture timeouts were recorded as harness warnings", html)


if __name__ == "__main__":
    unittest.main()
