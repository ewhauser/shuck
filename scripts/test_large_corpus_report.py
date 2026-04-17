import importlib.util
import sys
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
    def test_extract_main_report_body_ignores_sections_after_main_result(self) -> None:
        log = """large corpus compatibility summary: blocking=0 warnings=0 fixtures=1 unsupported_shells=0 implementation_diffs=0 mapping_issues=0 reviewed_divergences=0 corpus_noise=0 harness_warnings=0 harness_failures=0
test large_corpus_conforms_with_shellcheck ... ok
Harness Warnings:
/tmp/zsh-fixture.sh
  shuck timed out after 30.000s
"""

        self.assertIsNone(MODULE.extract_main_report_body(log))

    def test_extract_main_report_body_stays_within_main_test_span(self) -> None:
        log = """large corpus compatibility summary: blocking=0 warnings=1 fixtures=1 unsupported_shells=0 implementation_diffs=0 mapping_issues=1 reviewed_divergences=0 corpus_noise=0 harness_warnings=0 harness_failures=0
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


if __name__ == "__main__":
    unittest.main()
