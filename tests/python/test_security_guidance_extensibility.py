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
        dangerous = (
            r"(a+)*",
            r"(a*b)+",
            r"(.*)*",
            r"(a|aa)*",
            r"(ab|a)+",
            r"(?:cat|catalog)*",
            r"(?:a|aa){1000000000}$",
            r"(?:(?:a)|a){1000000000}$",
            r"(?i:a|A){1000000000}$",
            r"(?i:(a|A){1000000000})$",
            r"(?i)(a|A){1000000000}$",
            r"(?x:a| a){1000000000}$",
            r"(?x)(a| a){1000000000}$",
            r"(?:(?:a|aa)){1000000000}$",
            r"((?:a|aa)){1000000000}$",
            r"(?:(?:(?:a|aa))){2,1000000000}$",
            r"(a|)*",
            r"(?:|a)+",
            r"(a\|b|a\|bc)*",
            r"(?P<name>a|aa)*",
            r"(?i:a|aa)*",
            r"(?=a|aa)*",
            r"([)]a|[)]aa)*",
            "(" + ("+" * 20000) + ")*",
        )
        safe = (r"(a|b)*", r"(?:cat|dog)+", r"(?:a|aa){1}", r"^hello+$", r"foo\\|bar")

        for pattern in dangerous:
            with self.subTest(pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))
        for pattern in safe:
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))

    def test_rejects_custom_regexes_over_explicit_length_cap(self):
        pattern = "a" * (extensibility.CUSTOM_REGEX_MAX_CHARS + 1)

        self.assertIsNone(
            extensibility._validate_pattern(
                {"rule_name": "too-long", "reminder": "test", "regex": pattern},
                source="test",
            )
        )

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

        self.assertEqual(completed.stdout.strip(), "True")

    def test_nested_optional_groups_scan_within_deadline(self):
        code = f"""
import sys
sys.path.insert(0, {str(PLUGIN_SCRIPTS)!r})
import extensibility
pattern = ('(' * 20000) + 'a' + (')?' * 20000)
print(extensibility._has_redos_structure(pattern))
"""

        completed = subprocess.run(
            [sys.executable, "-c", code],
            timeout=1,
            check=True,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.stdout.strip(), "False")

    def test_nested_group_analysis_is_linear_within_regex_cap(self):
        code = f"""
import sys
sys.path.insert(0, {str(PLUGIN_SCRIPTS)!r})
import extensibility
pattern = '(a|' * 1000 + 'a' + ')' * 1000 + '{{2}}'
print(extensibility._has_redos_structure(pattern))
"""

        completed = subprocess.run(
            [sys.executable, "-c", code],
            timeout=1,
            check=True,
            capture_output=True,
            text=True,
        )

        self.assertEqual(completed.stdout.strip(), "True")

    def test_detector_does_not_use_alternation_regex_on_untrusted_input(self):
        source = (PLUGIN_SCRIPTS / "extensibility.py").read_text(encoding="utf-8")

        self.assertNotIn("_ALT_UNDER_REP", source)


if __name__ == "__main__":
    unittest.main()
