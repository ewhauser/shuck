import importlib.util
import sys
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).with_name("compact_large_corpus_log.py")
SPEC = importlib.util.spec_from_file_location("compact_large_corpus_log", SCRIPT_PATH)
assert SPEC is not None
assert SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class CompactLargeCorpusLogTests(unittest.TestCase):
    def test_keeps_high_signal_lines_and_drops_chatter(self) -> None:
        lines = [
            "Compiling shuck v0.0.7\n",
            "running 2 tests\n",
            "large corpus: processed 41/818 fixtures (5%)\n",
            "large corpus compatibility summary: blocking=1 warnings=2 fixtures=3 unsupported_shells=0 implementation_diffs=1 mapping_issues=1 reviewed_divergences=1 corpus_noise=0 harness_warnings=0 harness_failures=0\n",
            "test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 72 filtered out; finished in 267.03s\n",
        ]

        compacted = "".join(MODULE.iter_compacted_lines(lines))

        self.assertNotIn("Compiling shuck", compacted)
        self.assertIn("running 2 tests", compacted)
        self.assertIn("large corpus: processed 41/818 fixtures (5%)", compacted)
        self.assertIn("large corpus compatibility summary:", compacted)
        self.assertIn("test result: FAILED.", compacted)

    def test_truncates_large_sections_after_eight_blocks(self) -> None:
        lines = ["Implementation Diffs:\n"]
        for i in range(10):
            lines.extend(
                [
                    f"/tmp/fixture-{i}.sh\n",
                    f"  shellcheck-only C001/SC2000 {i + 1}:1-{i + 1}:5 error example {i}\n",
                    "\n",
                ]
            )
        lines.append("test large_corpus_conforms_with_shellcheck ... FAILED\n")

        compacted = "".join(MODULE.iter_compacted_lines(lines))

        for i in range(8):
            self.assertIn(f"/tmp/fixture-{i}.sh", compacted)
        self.assertNotIn("/tmp/fixture-8.sh", compacted)
        self.assertNotIn("/tmp/fixture-9.sh", compacted)
        self.assertIn("... omitted 2 additional entries from Implementation Diffs", compacted)
        self.assertIn("test large_corpus_conforms_with_shellcheck ... FAILED", compacted)

    def test_blank_lines_inside_one_fixture_do_not_hide_later_fixtures(self) -> None:
        lines = ["Implementation Diffs:\n", "/tmp/noisy.sh\n"]
        for i in range(8):
            lines.extend(
                [
                    f"  shellcheck-only C001/SC2000 {i + 1}:1-{i + 1}:5 error example {i}\n",
                    "\n",
                ]
            )
        lines.extend(
            [
                "/tmp/second.sh\n",
                "  shellcheck-only C001/SC2000 9:1-9:5 error second\n",
                "\n",
                "test large_corpus_conforms_with_shellcheck ... FAILED\n",
            ]
        )

        compacted = "".join(MODULE.iter_compacted_lines(lines))

        self.assertIn("/tmp/noisy.sh", compacted)
        self.assertIn("/tmp/second.sh", compacted)
        self.assertNotIn("additional entries from Implementation Diffs", compacted)


if __name__ == "__main__":
    unittest.main()
