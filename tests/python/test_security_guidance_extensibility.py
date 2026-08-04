import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PLUGIN_SCRIPTS = ROOT / "plugins" / "security-guidance" / "scripts"
sys.path.insert(0, str(PLUGIN_SCRIPTS))
import extensibility  # noqa: E402


class SecurityGuidanceExtensibilityTests(unittest.TestCase):
    def test_detects_documented_redos_categories(self):
        dangerous = (r"(a+)*", r"(a*b)+", r"(.*)*", r"(a|aa)*", r"(ab|a)+")
        safe = (r"(a|b)*", r"(?:cat|dog)+", r"^hello+$", r"foo\\|bar")

        for pattern in dangerous:
            with self.subTest(pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))
        for pattern in safe:
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))

    def test_pathological_alternation_scan_completes_within_deadline(self):
        code = f"""
import sys
sys.path.insert(0, {str(PLUGIN_SCRIPTS)!r})
import extensibility
pattern = '(||' + ('|' * 20000) + ')*'
print(extensibility._has_redos_structure(pattern))
"""

        completed = subprocess.run(
            [sys.executable, "-c", code],
            timeout=1,
            check=True,
            capture_output=True,
            text=True,
        )

        self.assertIn(completed.stdout.strip(), ("True", "False"))

    def test_detector_does_not_use_alternation_regex_on_untrusted_input(self):
        source = (PLUGIN_SCRIPTS / "extensibility.py").read_text(encoding="utf-8")

        self.assertNotIn("_ALT_UNDER_REP", source)


if __name__ == "__main__":
    unittest.main()
