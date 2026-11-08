# Live verdict changes under the strict class rules

Old host = `native_before` unless deferred, then the recorded script verdict. Old agent = recorded script verdict.
These are the two old live execution paths; deferred is an execution choice, never a verdict.
Non-finite first differences fail rather than receiving fabricated distinct tokens.
Identity strings compare exactly, with no case or whitespace folding.
The historical `python`, `native_before` and Round 2 `native` recordings remain unchanged.
`support/floor_strict_expectations.rs` asserts the intentional strict changes with exact findings.

Required version bumps (listed, not performed): **freeze-acceptance**, **verify-frozen-acceptance**.
Closure pins also need updating: freeze-skeleton, land-task-body, task-set-lint, requirements-trace and verify-frozen-skeleton (plus both bumped capabilities). No pin was changed.
Typed commands also change: a nonzero exit fails even with no JSON failure; predicates fail before commands run.
Malformed contracts (missing required metadata or unknown fields) now fail parsing on both host and CLI paths.
The review reports placeholder typed commands in four recent runs; that is not evidence that other floors are unchanged.
No new live-run audit of those four runs was performed in Round 3.

| Case | Old live host | Old live agent | New verdict | Class rule |
|---|---|---|---|---|
| `canonical_equivalent_floats` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `canonical_equivalent_series` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `canonical_float_and_integer` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `closed_date` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `closed_neither` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `closed_offset` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `closed_weekday` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `consistency_consistent` | passed (native) | passed | passed | D-S7-10: declared internal consistency is enforced |
| `consistency_inconsistent` | passed (native) | failed | failed | D-S7-10: declared internal consistency is enforced |
| `consistency_missing_checks` | passed (native) | passed | passed | D-S7-10: declared internal consistency is enforced |
| `consistency_not_array` | passed (native) | failed | failed | D-S7-10: declared internal consistency is enforced |
| `consistency_skipped` | passed (native) | passed | passed | D-S7-10: declared internal consistency is enforced |
| `contract_blank_optional` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `data_kind_complete` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `data_kind_missing_non_constant_fields` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `data_kind_missing_payload_path_field` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `data_kind_missing_required_fields` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `data_kind_missing_series_value_fields` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `data_kind_undeclared` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `data_kind_unsupported` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `extension_uppercase` | passed (native) | failed | failed | D-S7-10 / D-S7-8: ASCII case-insensitive extension inference |
| `gap_skip` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `glob_dotfile` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `glob_escaped_root` | failed (recorded script) | failed | passed | D-S7-7: root metacharacters are literal |
| `glob_first` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `glob_none` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `glob_two` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `identity_bool_numeric` | failed (native) | passed | failed | D-S7-10: identity types are strict; numbers compare numerically |
| `identity_float_numeric` | failed (native) | passed | passed | D-S7-10: identity types are strict; numbers compare numerically |
| `identity_numeric` | passed (native) | passed | passed | D-S7-10: identity types are strict; numbers compare numerically |
| `identity_string` | passed (native) | passed | passed | D-S7-10: identity types are strict; numbers compare numerically |
| `identity_type_mismatch` | passed (native) | failed | failed | D-S7-10: identity types are strict; numbers compare numerically |
| `instances_dict` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `instances_empty` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `instances_invalid` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `instances_list` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `instances_missing_file` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `instances_missing_ref` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `instances_not_collection` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `instances_text_empty` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `integer_bool` | failed (native) | passed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `integer_float` | failed (native) | passed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `integer_integer_float` | failed (native) | passed | passed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `integer_invalid` | failed (native) | failed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `integer_spaces` | passed (native) | passed | passed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `integer_zero` | failed (native) | failed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `json_directory_empty` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `json_directory_nonempty` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `json_infinity` | failed (native) | passed | failed | D-S7-10: non-finite numeric tokens are invalid JSON |
| `json_nan` | failed (native) | passed | failed | D-S7-10: non-finite numeric tokens are invalid JSON |
| `json_negative_infinity` | failed (native) | passed | failed | D-S7-10: non-finite numeric tokens are invalid JSON |
| `jsonl_cr` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `jsonl_lf` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `jsonl_unicode` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `minimum_zero_empty_object` | failed (native) | passed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `minimum_zero_missing` | failed (native) | passed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `minimum_zero_nonempty_object` | failed (native) | failed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `minimum_zero_null` | failed (native) | passed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `nonempty_empty` | failed (native) | failed | failed | D-S7-10: {} is empty |
| `nonempty_null` | failed (native) | failed | failed | D-S7-10: {} is empty |
| `nonempty_object` | failed (native) | passed | failed | D-S7-10: {} is empty |
| `nonempty_value` | passed (native) | passed | passed | D-S7-10: {} is empty |
| `payload_constant` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `payload_empty` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `payload_list` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `payload_missing_field` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `payload_object` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `payload_row_scalar` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `payload_unparsable` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `registry_bad_status` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `registry_missing` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `registry_not_object` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `registry_null` | failed (native) | passed | failed | D-S7-10: null registry/key fails |
| `registry_null_key` | failed (native) | passed | failed | D-S7-10: null registry/key fails |
| `registry_pass` | passed (native) | passed | passed | ported declared predicates; same recorded verdict |
| `request_below` | failed (recorded script) | failed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `request_bool` | passed (recorded script) | passed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `request_float` | passed (recorded script) | passed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `request_invalid` | failed (recorded script) | failed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `request_missing` | failed (recorded script) | failed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `request_pass` | passed (recorded script) | passed | passed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `request_spaces` | passed (recorded script) | passed | passed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `request_zero` | failed (recorded script) | failed | failed | D-S7-10: integral numbers or decimal integer strings; missing/null/objects/bools fail |
| `response_match` | passed (recorded script) | passed | passed | D-S7-10: identity types are strict; numbers compare numerically |
| `response_mismatch` | failed (recorded script) | failed | failed | D-S7-10: identity types are strict; numbers compare numerically |
| `response_missing` | failed (recorded script) | failed | failed | D-S7-10: identity types are strict; numbers compare numerically |
| `roots_first` | passed (native) | passed | passed | ported declared predicates; same recorded verdict |
| `roots_neither` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `roots_second` | passed (native) | passed | passed | ported declared predicates; same recorded verdict |
| `series_different` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `series_identical` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `series_nonfinite_infinity` | passed (recorded script) | passed | failed | D-S7-10: non-finite numeric tokens are invalid JSON |
| `series_nonfinite_nan` | passed (recorded script) | passed | failed | D-S7-10: non-finite numeric tokens are invalid JSON |
| `series_nonfinite_negative_infinity` | passed (recorded script) | passed | failed | D-S7-10: non-finite numeric tokens are invalid JSON |
| `series_overlap` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `series_short` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `series_steps` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `single_directory_empty` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `single_directory_nonempty` | passed (native) | passed | passed | ported declared predicates; same recorded verdict |
| `single_empty` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `single_list` | passed (native) | passed | passed | ported declared predicates; same recorded verdict |
| `single_missing` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `single_null` | passed (native) | failed | failed | D-S7-10: null artifact fails |
| `single_present` | passed (native) | passed | passed | ported declared predicates; same recorded verdict |
| `single_scalar` | passed (native) | passed | passed | ported declared predicates; same recorded verdict |
| `steps_bools` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `steps_constant` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `steps_low` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `steps_repeated_infinity` | passed (recorded script) | passed | failed | D-S7-10: non-finite numeric tokens are invalid JSON |
| `steps_repeated_nan` | passed (recorded script) | passed | failed | D-S7-10: non-finite numeric tokens are invalid JSON |
| `steps_repeated_negative_infinity` | passed (recorded script) | passed | failed | D-S7-10: non-finite numeric tokens are invalid JSON |
| `steps_short` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `steps_strings` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `steps_varied` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `steps_zero_override` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |
| `template_typed` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `template_unbound` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `template_universe` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `text_declared` | passed (native) | passed | passed | ported declared predicates; same recorded verdict |
| `text_inferred` | passed (native) | passed | passed | ported declared predicates; same recorded verdict |
| `text_unsupported` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `timestamp_compact` | passed (recorded script) | passed | failed | D-S7-6: declared timestamp grammar; UTC comparison |
| `timestamp_date` | passed (recorded script) | passed | passed | D-S7-6: declared timestamp grammar; UTC comparison |
| `timestamp_iso_z` | failed (recorded script) | failed | failed | D-S7-6: declared timestamp grammar; UTC comparison |
| `timestamp_millis` | failed (recorded script) | failed | failed | D-S7-6: declared timestamp grammar; UTC comparison |
| `timestamp_naive` | passed (recorded script) | passed | passed | D-S7-6: declared timestamp grammar; UTC comparison |
| `timestamp_offset` | passed (recorded script) | passed | passed | D-S7-6: declared timestamp grammar; UTC comparison |
| `timestamp_seconds` | failed (recorded script) | failed | failed | D-S7-6: declared timestamp grammar; UTC comparison |
| `timestamp_unparsable` | failed (recorded script) | failed | failed | D-S7-6: declared timestamp grammar; UTC comparison |
| `universe_duplicate` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `universe_empty_axis` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `universe_extra` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `universe_missing` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `universe_not_array` | failed (native) | failed | failed | ported declared predicates; same recorded verdict |
| `universe_pass` | passed (native) | passed | passed | ported declared predicates; same recorded verdict |
| `validation_inconsistent` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `validation_missing_file` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `validation_missing_ref` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `validation_not_passed` | failed (recorded script) | failed | failed | ported declared predicates; same recorded verdict |
| `validation_pass` | passed (recorded script) | passed | passed | ported declared predicates; same recorded verdict |

Moved-test inventory: `floor_test_moves.json`. The adapter now executes typed commands and parses the emitted contract unchanged.
Three source fixtures in `deliverable_contract_tests.rs` now explicitly declare `kind`: single-root missing, repository JSON and glob instances.
Their evidence bytes and predicate assertions are unchanged. The missing-kind variant is separately asserted to fail parsing in `deliverable_native_tests.rs`.
Cleanup carried from Round 2: `acceptance_scratch_confine.rs` and `acceptance_scratch_mutation_fd.rs` scope imports to their platform;
`store_private.rs` scopes the mutable directory builder to the platform that sets its mode.
Three existing test-helper re-exports in `workflow_acceptance_executability.rs` and its `verdict` module are scoped to their Unix-only callers; no test was gated out.
