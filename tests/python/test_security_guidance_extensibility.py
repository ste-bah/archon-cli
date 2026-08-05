import subprocess
import sys
import unittest
from unittest import mock
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

    def test_rejects_adjacent_variable_quantifiers_fail_closed(self):
        patterns = (
            r"a*a*a*a*a*a*a*a!",
            r"(?:a*a*)!",
            r"[ab]*[cd]*!",
            r"[a-c]*[d-f]*!",
            r"\d*\s*!",
            r"a*b*!",
            r"(?:a)*(?:b)*!",
            r"a*(?:a)*!",
            r"(?i)a*(?:A)*!",
            r"a*?a*!",
            r"a{1,999999}a{1,999999}!",
            r"a*+b*",
            r"a++b++",
            r"a{1,9}+b{2,}+",
            r"[\d]*1*!",
            r"(?:a|b)*a*!",
        )

        for pattern in patterns:
            with self.subTest(pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))
                self.assertIsNone(
                    extensibility._validate_pattern(
                        {"rule_name": "adjacent", "reminder": "test", "regex": pattern},
                        source="test",
                    )
                )

    def test_rejects_adjacent_variable_quantifiers_in_verbose_mode(self):
        unsafe = (
            "(?x:a* a*)$",
            "(?x:a* # comment\n a*)$",
            "(?x)a* # comment\n a*$",
            "(?x:[a]* # comment\n [b]*)$",
            "(?-x:(?x:a* # nested comment\n a*))$",
        )

        for pattern in unsafe:
            with self.subTest(pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))
                with mock.patch.object(extensibility.re, "compile") as compile_mock:
                    self.assertIsNone(
                        extensibility._validate_pattern(
                            {"rule_name": "verbose", "reminder": "test", "regex": pattern},
                            source="test",
                        )
                    )
                    compile_mock.assert_not_called()

    def test_rejects_verbose_gaps_between_atoms_and_quantifiers(self):
        unsafe = (
            "(?x)a * b *$",
            "(?x:a * b *)$",
            "(?-x:(?x:a * b *))$",
            "(?x:a\t*\tb\t*)$",
            "(?x:a\n*\nb\n*)$",
            "(?x:a\r\n*\r\nb\r\n*)$",
            "(?x:a # first\n * b # second\n *)$",
            "(?x:\\d * \\s *)$",
            "(?x:\\x61 * \\N{LATIN SMALL LETTER B} *)$",
            "(?x:[ab] * [cd] *)$",
            "(?x:a + b +)$",
            "(?x:a *? b *?)$",
            "(?x:a *+ b *+)$",
            "(?x:a {1,9} b {2,})$",
            "(?x:a {1,9}? b {2,}+)$",
            "(?x:(?=a * b *))$",
            "(?x:(?!a * b *))$",
            "(?x:(?>a * b *))$",
            "(?x)(a)?(?(1)a * b *|c * d *)$",
        )

        for pattern in unsafe:
            with self.subTest(pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))
                with mock.patch.object(extensibility.re, "compile") as compile_mock:
                    self.assertIsNone(
                        extensibility._validate_pattern(
                            {"rule_name": "verbose-gap", "reminder": "test", "regex": pattern},
                            source="test",
                        )
                    )
                    compile_mock.assert_not_called()

    def test_verbose_atom_quantifier_gaps_respect_significant_literals(self):
        safe = (
            r"(?x:a\ *b\ *)$",
            r"(?x:a\#*b\#*)$",
            "(?x:(?-x:a * b *))$",
            "(?x:(?-x:a#*b#*))$",
        )

        for pattern in safe:
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(
                    extensibility._validate_pattern(
                        {"rule_name": "verbose-gap-safe", "reminder": "test", "regex": pattern},
                        source="test",
                    )
                )

    def test_rejects_verbose_group_quantifiers_after_insignificant_syntax(self):
        patterns = (
            "(?x:(?:a) * # q1\n (?:a) *)$",
            "(?x:(?:a) # q1\n * (?:a) *)$",
            "(?x:(?:a)\r\n * (?:a) *)$",
            "(?x:(?:a)# note\n*(?:b)*)$",
        )

        for pattern in patterns:
            with self.subTest(pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))
                with mock.patch.object(extensibility.re, "compile") as compile_mock:
                    self.assertIsNone(
                        extensibility._validate_pattern(
                            {"rule_name": "verbose-group", "reminder": "test", "regex": pattern},
                            source="test",
                        )
                    )
                    compile_mock.assert_not_called()

    def test_rejects_adjacent_variable_atoms_across_unquantified_groups(self):
        patterns = (
            r"(a*)a*$",
            r"a*(?:a*)$",
            r"(?:a*)(?:b*)$",
            "(?x:a*(?:a* # inner\n ))$",
            "(?x:(?:a*) # outer\n a*)$",
        )

        for pattern in patterns:
            with self.subTest(pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))
                with mock.patch.object(extensibility.re, "compile") as compile_mock:
                    self.assertIsNone(
                        extensibility._validate_pattern(
                            {"rule_name": "group-boundary", "reminder": "test", "regex": pattern},
                            source="test",
                        )
                    )
                    compile_mock.assert_not_called()

    def _assert_rejected_before_compile(self, pattern, rule_name):
        with mock.patch.object(extensibility.re, "compile") as compile_mock:
            result = extensibility._validate_pattern(
                {"rule_name": rule_name, "reminder": "test", "regex": pattern},
                source="test",
            )
        compile_mock.assert_not_called()
        self.assertTrue(extensibility._has_redos_structure(pattern))
        self.assertIsNone(result)

    def test_rejects_adjacent_optional_quantifier_forms(self):
        unsafe = (
            r"a?a?", r"a??b??", r"a?+b?+", r"a{0,1}b{1,2}",
            r"a{0,1}?b{1,2}+", "(?x:a ? # gap\n b ??)",
        )

        for pattern in unsafe:
            with self.subTest(pattern=pattern):
                self._assert_rejected_before_compile(pattern, "optional")

    def test_character_classes_with_leading_closing_brackets(self):
        unsafe = (
            r"[]a]*[]a]*!", r"[^]a]*[^]a]*!", r"[]a]*[\]b]*!",
            r"[]\\]*[]\\]*!", "(?x:[]a] * # gap\n [^]b] *)",
        )
        safe = (r"[]a]*b", r"[^]a]{2}b*", r"[\]]*b")

        for pattern in unsafe:
            with self.subTest(kind="unsafe", pattern=pattern):
                self._assert_rejected_before_compile(pattern, "leading-bracket")
        for pattern in safe:
            with self.subTest(kind="safe", pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(extensibility._validated_regex("leading-bracket-safe", pattern))

    def test_groups_preserve_variable_interiors_before_fixed_atoms(self):
        unsafe = (
            r"(a*a)a*!", r"(?:a*a|b)a*!", r"((a*)a)a*!",
            r"(?:a|b*b)c*!", "(?x:(a* a) # gap\n a*)!",
        )
        safe = (r"(aa)a*!", r"(?:aa|b)a*!", r"(?=a*a)a*!")

        for pattern in unsafe:
            with self.subTest(kind="unsafe", pattern=pattern):
                self._assert_rejected_before_compile(pattern, "group-interior")
        for pattern in safe:
            with self.subTest(kind="safe", pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(extensibility._validated_regex("group-interior-safe", pattern))

    def test_zero_width_escape_assertions_preserve_adjacency(self):
        unsafe = (
            r"a*\ba*!", r"a*\Ba*!", r"a*\Aa*!", r"a*\Za*!",
            "(?x:a* \\b # gap\n a*)!", r"(a)a*\1*!", r"a*\d*!",
        )
        safe = (r"a*\bb", r"a*\Bd", r"a*\dc", r"(a)a*\1b")

        for pattern in unsafe:
            with self.subTest(kind="unsafe", pattern=pattern):
                self._assert_rejected_before_compile(pattern, "escaped-assertion")
        for pattern in safe:
            with self.subTest(kind="safe", pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(extensibility._validated_regex("escaped-assertion-safe", pattern))

    def test_repeated_groups_include_optional_quantifiers_and_leading_bracket_classes(self):
        unsafe = (
            r"(a?){30}a{30}$", r"(a??)+X", r"(a?+){2,}X",
            r"([])]*)+X", r"([^])]*)+X", r"([]\\]*)+X",
        )

        for pattern in unsafe:
            with self.subTest(pattern=pattern):
                self._assert_rejected_before_compile(pattern, "repeated-edge")

    def test_repeated_group_summary_respects_fixed_and_zero_width_interiors(self):
        safe = (
            r"(?:ab{2}){3}", r"(a{1}){2}", r"(?:a{2}?b){3}",
            r"(?:a{2}+b){3}", r"(?:(?=a*)b){2}", r"(?:(?<=a{2})b){2}",
        )

        for pattern in safe:
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(extensibility._validated_regex("fixed-interior", pattern))

    def test_comment_groups_do_not_reset_adjacent_variable_atom_state(self):
        patterns = (
            r"a*(?#comment)a*$",
            r"a*(?# separator)b*$",
            r"(?x:a*(?#comment)a*)$",
            r"(?x:a*(?#escaped \#)a*)$",
        )

        for pattern in patterns:
            with self.subTest(pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))
                with mock.patch.object(extensibility.re, "compile") as compile_mock:
                    self.assertIsNone(
                        extensibility._validate_pattern(
                            {"rule_name": "comment-group", "reminder": "test", "regex": pattern},
                            source="test",
                        )
                    )
                    compile_mock.assert_not_called()

    def test_comment_groups_preserve_adjacency_through_escaped_parentheses(self):
        unsafe = (
            r"a*(?#foo\)bar)b*$",
            r"a*(?#foo\\\)bar)b*$",
            r"(?x:a*(?#foo\)bar)b*)$",
            r"a*(?#one\)two\)three)b*$",
        )
        safe = (
            r"a*(?#foo\\)b$",
            r"a*(?#foo\\\\)b$",
            r"a*(?#one\)two)b$",
        )

        for pattern in unsafe:
            with self.subTest(kind="unsafe", pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))
                with mock.patch.object(extensibility.re, "compile") as compile_mock:
                    self.assertIsNone(
                        extensibility._validate_pattern(
                            {"rule_name": "comment-escape", "reminder": "test", "regex": pattern},
                            source="test",
                        )
                    )
                    compile_mock.assert_not_called()
        for pattern in safe:
            with self.subTest(kind="safe", pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(
                    extensibility._validate_pattern(
                        {"rule_name": "comment-escape-safe", "reminder": "test", "regex": pattern},
                        source="test",
                    )
                )

    def test_unclosed_comment_groups_reach_compiler_rejection(self):
        for pattern in (r"a*(?#unterminated", r"a*(?#unterminated\)"):
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNone(
                    extensibility._validate_pattern(
                        {"rule_name": "comment-invalid", "reminder": "test", "regex": pattern},
                        source="test",
                    )
                )

    def test_verbose_mode_respects_scopes_classes_and_escaped_literals(self):
        safe = (
            "(?x:a*(?-x: )a*)$",
            "(?-x:a*(?x: # nested comment\n b))$",
            "(?x:[ #]*)$",
            r"(?x:\ *)$",
            r"(?x:\#*)$",
            "(?x:a* # one variable quantifier\n )$",
            r"(?=a*)b*$",
            r"(?:a*|b)$",
            r"(?:ab){2}c*$",
            r"a*(?#comment)b$",
            "(?x:(?:a) # fixed repeat\n {2})$",
            "(?x:(?:a) # CRLF fixed repeat\r\n {2})$",
        )

        for pattern in safe:
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(
                    extensibility._validate_pattern(
                        {"rule_name": "verbose-safe", "reminder": "test", "regex": pattern},
                        source="test",
                    )
                )

    def test_verbose_comments_do_not_trigger_repeated_group_detection(self):
        patterns = (
            "(?x:a # +\n)*",
            "(?x:(?# +)a)*",
        )

        for pattern in patterns:
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(
                    extensibility._validate_pattern(
                        {"rule_name": "verbose-repeat-safe", "reminder": "test", "regex": pattern},
                        source="test",
                    )
                )

    def test_escaped_atom_end_consumes_python_multi_character_escapes(self):
        for escaped in (
            r"\N{LATIN CAPITAL LETTER A}",
            "\\" + "u0041",
            r"\U00000041",
            r"\x41",
            r"\101",
            r"\1",
        ):
            with self.subTest(escaped=escaped):
                self.assertEqual(extensibility._escaped_atom_end(escaped, 0), len(escaped))

    def test_rejects_adjacent_variable_quantifiers_for_escaped_atoms(self):
        patterns = (
            r"\N{LATIN CAPITAL LETTER A}*\d*",
            "\\" + "u0041*\\d*",
            r"\U00000041*\d*",
            r"\x41*\d*",
            r"\101*\d*",
            r"(a)\1*\d*",
            r"a{,3}a*",
            r"a{,}a*",
            r"(a+){,}",
            r"([\])]+)*",
            r"^(a)?((?(1)b+|b))*$",
            "(?x:(a+ # )\n)*",
        )

        for pattern in patterns:
            with self.subTest(pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))
                self.assertIsNone(
                    extensibility._validate_pattern(
                        {"rule_name": "escaped", "reminder": "test", "regex": pattern},
                        source="test",
                    )
                )

    def test_repeated_group_scanner_respects_conditional_groups_and_verbose_comments(self):
        for pattern in (r"^(a)?((?(1)b+|b))*$", "(?x:(a+ # )\n)*"):
            with self.subTest(pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))

    def test_allows_nested_repetition_with_outer_maximum_one(self):
        pattern = r"(a+){,1}"

        self.assertFalse(extensibility._has_redos_structure(pattern))
        self.assertIsNotNone(
            extensibility._validate_pattern(
                {"rule_name": "bounded", "reminder": "test", "regex": pattern}, source="test"
            )
        )

    def test_nested_repetition_respects_bounded_outer_maximum(self):
        safe = (r"(a+){,0}", r"(a+){,1}", r"(a+){0,1}", r"(a+){1,1}")
        unsafe = (r"(a+){,2}", r"(a+){1,2}", r"(a+){,}", r"(a+){2,}")

        for pattern in safe:
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(
                    extensibility._validate_pattern(
                        {"rule_name": "bounded", "reminder": "test", "regex": pattern}, source="test"
                    )
                )
        for pattern in unsafe:
            with self.subTest(pattern=pattern):
                self.assertTrue(extensibility._has_redos_structure(pattern))
                self.assertIsNone(
                    extensibility._validate_pattern(
                        {"rule_name": "bounded", "reminder": "test", "regex": pattern}, source="test"
                    )
                )

    def test_allows_single_variable_quantifiers_for_escaped_atoms(self):
        patterns = (
            r"\N{LATIN CAPITAL LETTER A}*",
            "\\" + "u0041*",
            r"\U00000041*",
            r"\x41*",
            r"\101*",
            r"(a)\1*",
        )

        for pattern in patterns:
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(
                    extensibility._validate_pattern(
                        {"rule_name": "escaped", "reminder": "test", "regex": pattern},
                        source="test",
                    )
                )

    def test_rejects_malformed_multi_character_escapes_without_scanner_errors(self):
        for pattern in (r"\N{unterminated", r"\x4", r"\u041", r"\U0000004"):
            with self.subTest(pattern=pattern):
                self.assertIsNone(
                    extensibility._validate_pattern(
                        {"rule_name": "invalid", "reminder": "test", "regex": pattern}, source="test"
                    )
                )

    def test_rejects_unrepresentable_repeat_bounds_without_crashing(self):
        enormous_bound = "9" * 30

        for pattern in (f"a{{{enormous_bound}}}", f"a{{0,{enormous_bound}}}"):
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNone(
                    extensibility._validate_pattern(
                        {"rule_name": "overflow", "reminder": "test", "regex": pattern},
                        source="test",
                    )
                )

    def test_rejects_compiler_recursion_error_without_crashing(self):
        pattern = r"safe-pattern"

        self.assertFalse(extensibility._has_redos_structure(pattern))
        with mock.patch.object(
            extensibility.re, "compile", side_effect=RecursionError("compiler recursion")
        ):
            self.assertIsNone(
                extensibility._validate_pattern(
                    {"rule_name": "recursion", "reminder": "test", "regex": pattern}, source="test"
                )
            )

    def test_accepts_ordinary_nested_groups(self):
        ordinarily_nested = "(" * 100 + "a" + ")" * 100

        self.assertFalse(extensibility._has_redos_structure(ordinarily_nested))
        self.assertIsNotNone(
            extensibility._validate_pattern(
                {"rule_name": "nested", "reminder": "test", "regex": ordinarily_nested}, source="test"
            )
        )

    def test_invalid_and_fixed_repeat_bounds_remain_compiler_validated(self):
        self.assertIsNone(
            extensibility._validate_pattern(
                {"rule_name": "reversed", "reminder": "test", "regex": r"a{2,1}"}, source="test"
            )
        )

        for pattern in (r"a{2}b{2}", r"(?:ab){2}c{2}"):
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(
                    extensibility._validate_pattern(
                        {"rule_name": "fixed", "reminder": "test", "regex": pattern}, source="test"
                    )
                )

    def test_allows_single_variable_quantifiers_and_invalid_syntax_fails_closed(self):
        for pattern in (r"a*", r"a+?", r"a{1,1}", r"^hello+$", r"(?:cat|dog)+", r"(?m)(a|b)*"):
            with self.subTest(pattern=pattern):
                self.assertFalse(extensibility._has_redos_structure(pattern))
                self.assertIsNotNone(
                    extensibility._validate_pattern(
                        {"rule_name": "single", "reminder": "test", "regex": pattern}, source="test"
                    )
                )

        self.assertFalse(extensibility._has_redos_structure(r"a*)"))
        self.assertIsNone(
            extensibility._validate_pattern(
                {"rule_name": "invalid", "reminder": "test", "regex": r"a*)"}, source="test"
            )
        )

        pattern = "a" * (extensibility.CUSTOM_REGEX_MAX_CHARS + 1)

        self.assertIsNone(
            extensibility._validate_pattern(
                {"rule_name": "too-long", "reminder": "test", "regex": pattern},
                source="test",
            )
        )

    def test_rejects_three_way_repeated_alternation_without_branch_storage(self):
        pattern = r"(?:a|b|c)*"

        self.assertTrue(extensibility._has_redos_structure(pattern))
        with mock.patch.object(extensibility.re, "compile") as compile_mock:
            self.assertIsNone(
                extensibility._validate_pattern(
                    {"rule_name": "alternation", "reminder": "test", "regex": pattern},
                    source="test",
                )
            )
            compile_mock.assert_not_called()

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

    def test_adjacent_quantifier_scan_completes_within_deadline(self):
        code = f"""
import sys
sys.path.insert(0, {str(PLUGIN_SCRIPTS)!r})
import extensibility
pattern = ('[ab]*' * 2000) + '!'
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

    def test_detector_uses_no_semantic_overlap_machinery(self):
        source = (PLUGIN_SCRIPTS / "extensibility.py").read_text(encoding="utf-8")

        for symbol in ("_radix_sort_ranges", "_class_ranges", "_atoms_overlap", "_category_overlap"):
            with self.subTest(symbol=symbol):
                self.assertNotIn(symbol, source)

    def test_detector_does_not_use_alternation_regex_on_untrusted_input(self):
        source = (PLUGIN_SCRIPTS / "extensibility.py").read_text(encoding="utf-8")

        self.assertNotIn("_ALT_UNDER_REP", source)


if __name__ == "__main__":
    unittest.main()
