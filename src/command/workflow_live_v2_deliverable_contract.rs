use serde_json::Value;

pub(super) fn verification_command(root: &str, contract: &Value) -> String {
    let root_literal = serde_json::to_string(root).expect("project root JSON");
    let contract_json = serde_json::to_string(contract).expect("deliverable contract JSON");
    let contract_literal =
        serde_json::to_string(&contract_json).expect("deliverable contract JSON literal");
    let verifier = VERIFIER
        .replace("__PROJECT_ROOT__", &root_literal)
        .replace("__CONTRACT_JSON__", &contract_literal);
    let Some(command) = typed_verification_command(root, contract) else {
        return verifier;
    };
    format!("{command}\n{verifier}")
}

pub(super) fn typed_verification_command(root: &str, contract: &Value) -> Option<String> {
    let command = contract.get("typed_verifier_command")?.as_str()?.trim();
    if command.is_empty() {
        return None;
    }
    let artifact = resolve_contract_path(root, contract.get("artifact_path"));
    let registry = resolve_contract_path(root, contract.get("registry_path"));
    Some(
        command
            .replace("{artifact_path}", &shell_quote(&artifact))
            .replace("{registry_path}", &shell_quote(&registry)),
    )
}

fn resolve_contract_path(root: &str, value: Option<&Value>) -> String {
    let value = value.and_then(Value::as_str).unwrap_or_default();
    let path = std::path::Path::new(value);
    if path.is_absolute() {
        value.to_string()
    } else {
        std::path::Path::new(root).join(path).display().to_string()
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

const VERIFIER: &str = r#"python3 - <<'PY'
import hashlib
import itertools
import json
import pathlib
import sys

root = pathlib.Path(__PROJECT_ROOT__)
contract = json.loads(__CONTRACT_JSON__)
failures = []

def resolve(value):
    path = pathlib.Path(value)
    return path if path.is_absolute() else root / path

def get_field(value, path, default=None):
    if not path:
        return default
    current = value
    for part in str(path).split('.'):
        if not isinstance(current, dict) or part not in current:
            return default
        current = current[part]
    return current

def normalized(value):
    return str(value if value is not None else '').strip().lower()

def load_json(path, label):
    if not path.is_file() or path.stat().st_size == 0:
        failures.append(f'{label} missing or empty: {path}')
        return None
    try:
        return json.loads(path.read_text())
    except Exception as error:
        failures.append(f'{label} is not valid JSON: {path}: {error}')
        return None

def load_payload(path, payload_format):
    if not path.is_file() or path.stat().st_size == 0:
        failures.append(f'payload missing or empty: {path}')
        return []
    try:
        if payload_format == 'jsonl':
            return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
        value = json.loads(path.read_text())
        return value if isinstance(value, list) else [value]
    except Exception as error:
        failures.append(f'payload cannot be parsed as {payload_format}: {path}: {error}')
        return []

def internal_consistency(value, label):
    if not isinstance(value, dict):
        return
    status_field = contract.get('validation_status_field')
    checks_field = contract.get('validation_checks_field')
    check_status_field = contract.get('validation_check_status_field')
    failed_values = {normalized(item) for item in contract.get('validation_failed_values', [])}
    passed_values = {normalized(item) for item in contract.get('validation_passed_values', [])}
    if not status_field or not checks_field or not check_status_field:
        return
    checks = get_field(value, checks_field, [])
    if not isinstance(checks, list):
        failures.append(f'{label} checks field is not an array')
        return
    failed_checks = [
        check for check in checks
        if normalized(get_field(check, check_status_field)) in failed_values
    ]
    overall = normalized(get_field(value, status_field))
    if failed_checks and overall in passed_values:
        failures.append(
            f'{label} is internally inconsistent: overall={overall} with {len(failed_checks)} failed check(s)'
        )

import glob as _glob
import re as _re

def has_placeholder(text):
    return '<' in str(text) and '>' in str(text)

def source_instances(art_field, source_path_value, records_field):
    # Precise per-instance resolution: instances are the entries of a declared
    # source-of-truth collection (e.g. the registry). Each entry names its own
    # artifact via art_field. Ignores filesystem orphans (files not backed by an
    # entry are not THIS contract's deliverables); detects entries whose artifact
    # is missing. Zero entries -> zero required instances (vacuous).
    source = load_json(resolve(source_path_value), 'declared instance source')
    records = get_field(source, records_field, {}) if source else {}
    if isinstance(records, dict):
        record_iter = list(records.items())
    elif isinstance(records, list):
        record_iter = list(enumerate(records))
    else:
        failures.append(f'instance source records field is not a collection: {records_field}')
        record_iter = []
    paths = []
    for key, record in record_iter:
        ref = get_field(record, art_field) if isinstance(record, dict) else None
        if not ref:
            failures.append(f'source entry {key} has no artifact reference field {art_field}')
            continue
        paths.append(resolve(ref))
    return paths

def glob_instances(pattern_text):
    # Fallback when no source collection is declared: every <segment> is a
    # wildcard. Weaker than source-bound (cannot detect a missing instance,
    # and will inspect filesystem orphans) but still generic and fail-closed.
    pattern = _re.sub(r'<[^>]+>', '*', str(pattern_text))
    base = pathlib.Path(pattern)
    concrete = str(base) if base.is_absolute() else str(root / base)
    return sorted(pathlib.Path(match) for match in _glob.glob(concrete))

# Parameterized deliverable path (e.g. .../<dataset-id>/<version>/x.json): one
# artifact per runtime-determined instance. Generic across any PRD. Prefer a
# declared source collection; fall back to glob. Zero instances is vacuously
# satisfied unless the contract declares min_instances. required_universe
# contracts keep the single-file path below (their instances are enumerated via
# the registry records loop already present further down).
raw_artifact_path = str(contract.get('artifact_path') or '')
if has_placeholder(raw_artifact_path) and not contract.get('required_universe'):
    art_field = contract.get('instance_artifact_field')
    source_path_value = contract.get('instance_source_path') or contract.get('registry_path')
    records_field = contract.get('instance_source_records_field') or contract.get('registry_records_field')
    if art_field and source_path_value and records_field:
        instance_paths = source_instances(art_field, source_path_value, records_field)
    else:
        instance_paths = glob_instances(raw_artifact_path)
    try:
        min_instances = int(contract.get('min_instances', 0) or 0)
    except (TypeError, ValueError):
        min_instances = 0
    if len(instance_paths) < min_instances:
        failures.append(
            f'declared deliverable requires >= {min_instances} instance(s), '
            f'found {len(instance_paths)} for {raw_artifact_path}'
        )
    for instance in instance_paths:
        instance_artifact = load_json(instance, f'declared deliverable instance {instance}')
        if instance_artifact is not None:
            internal_consistency(instance_artifact, f'declared deliverable instance {instance}')
    if failures:
        print(json.dumps({'failures': failures}, indent=2))
        raise SystemExit(1)
    print(json.dumps({
        'status': 'declared_deliverable_instances_present',
        'pattern': raw_artifact_path,
        'instance_count': len(instance_paths),
    }))
    raise SystemExit(0)

artifact_path = resolve(contract['artifact_path'])
artifact = load_json(artifact_path, 'declared deliverable')
if artifact is None:
    print(json.dumps({'failures': failures}, indent=2))
    raise SystemExit(1)
internal_consistency(artifact, 'declared deliverable')

registry = None
if contract.get('registry_path'):
    registry = load_json(resolve(contract['registry_path']), 'declared registry')

data_kind = str(contract.get('data_kind') or '').strip()
payload_declarations = any([
    contract.get('payload_path_field'),
    contract.get('required_fields'),
    contract.get('non_constant_fields'),
    contract.get('series_value_fields'),
])
if data_kind and data_kind != 'record_series':
    failures.append(f'unsupported declared data_kind: {data_kind}')
if payload_declarations and data_kind != 'record_series':
    failures.append('payload substance declarations require data_kind=record_series')
if data_kind == 'record_series':
    if not contract.get('payload_path_field'):
        failures.append('record_series contract has no payload_path_field')
    if not contract.get('required_fields'):
        failures.append('record_series contract has no required_fields')
    if not contract.get('non_constant_fields'):
        failures.append('record_series contract has no non_constant_fields')
    if not contract.get('series_value_fields'):
        failures.append('record_series contract has no series_value_fields')

if not contract.get('required_universe'):
    if failures:
        print(json.dumps({'failures': failures}, indent=2))
        raise SystemExit(1)
    print(json.dumps({'status': 'declared_deliverable_present', 'artifact': str(artifact_path)}))
    raise SystemExit(0)

cells_field = contract.get('cells_field')
cells = get_field(artifact, cells_field, [])
if not isinstance(cells, list):
    failures.append(f'declared cells field is not an array: {cells_field}')
    cells = []
identity_fields = contract.get('cell_identity_fields', [])
if not identity_fields:
    failures.append('required-universe contract has no cell_identity_fields')

universe_fields = contract.get('universe_fields', [])
axes = [get_field(artifact, field, []) for field in universe_fields]
if any(not isinstance(axis, list) or not axis for axis in axes):
    failures.append('declared required universe has an empty or non-array axis')
    required = set()
else:
    required = set(itertools.product(*axes))
indexed = {}
for cell in cells:
    identity = tuple(get_field(cell, field) for field in identity_fields)
    if identity in indexed:
        failures.append(f'duplicate cell identity: {identity}')
    indexed[identity] = cell

if required and set(indexed) != required:
    identity_sort_key = lambda identity: tuple(str(item) for item in identity)
    for identity in sorted(required - set(indexed), key=identity_sort_key):
        failures.append(f'missing required cell: {identity}')
    for identity in sorted(set(indexed) - required, key=identity_sort_key):
        failures.append(f'extra undeclared cell: {identity}')

gaps_field = contract.get('gaps_field')
gaps = get_field(artifact, gaps_field, []) if gaps_field else []
if gaps:
    failures.append(f'declared deliverable contains {len(gaps)} gap record(s)')
gap_identities = {
    tuple(get_field(gap, field) for field in identity_fields)
    for gap in gaps if isinstance(gap, dict)
}

records = get_field(registry, contract.get('registry_records_field'), {}) if registry else {}
if registry is not None and not isinstance(records, dict):
    failures.append('declared registry records field is not an object')
    records = {}

series = []
step_series = []
for identity, cell in sorted(
    indexed.items(),
    key=lambda item: tuple(str(part) for part in item[0]),
):
    label = ':'.join(str(item) for item in identity)
    for field in contract.get('required_true_fields', []):
        if get_field(cell, field) is not True:
            failures.append(f'{label} required true field failed: {field}')
    if identity in gap_identities:
        continue
    for field in contract.get('required_nonempty_fields', []):
        if get_field(cell, field) in (None, '', []):
            failures.append(f'{label} required non-empty field failed: {field}')
    for field in contract.get('positive_count_fields', []):
        try:
            if int(get_field(cell, field, 0) or 0) <= 0:
                failures.append(f'{label} positive count field failed: {field}')
        except (TypeError, ValueError):
            failures.append(f'{label} positive count field is not numeric: {field}')

    key_values = [get_field(cell, field) for field in contract.get('registry_key_fields', [])]
    key = ':'.join(str(value) for value in key_values)
    record = records.get(key) if key else None
    if registry is not None and not isinstance(record, dict):
        failures.append(f'{label} has no declared registry record for key {key}')
        continue
    if record is None:
        record = {}

    for field in contract.get('registry_required_true_fields', []):
        if get_field(record, field) is not True:
            failures.append(f'{label} registry required true field failed: {field}')
    status_field = contract.get('registry_status_field')
    allowed_statuses = {
        normalized(value) for value in contract.get('registry_allowed_statuses', [])
    }
    if status_field and normalized(get_field(record, status_field)) not in allowed_statuses:
        failures.append(f'{label} registry status is not allowed')
    count_field = contract.get('registry_count_field')
    if count_field:
        try:
            if int(get_field(record, count_field, 0) or 0) <= 0:
                failures.append(f'{label} registry count is not positive: {count_field}')
        except (TypeError, ValueError):
            failures.append(f'{label} registry count is not numeric: {count_field}')
    for cell_field, registry_field in contract.get('registry_identity_fields', {}).items():
        if normalized(get_field(cell, cell_field)) != normalized(get_field(record, registry_field)):
            failures.append(
                f'{label} registry identity mismatch: {cell_field}!={registry_field}'
            )

    validation_path_field = contract.get('validation_path_field')
    if validation_path_field:
        validation_ref = get_field(record, validation_path_field)
        if not validation_ref:
            failures.append(f'{label} registry has no validation path')
        else:
            validation = load_json(resolve(validation_ref), f'{label} validation')
            internal_consistency(validation, f'{label} validation')
            if isinstance(validation, dict):
                overall = normalized(
                    get_field(validation, contract.get('validation_status_field'))
                )
                passed = {
                    normalized(value)
                    for value in contract.get('validation_passed_values', [])
                }
                if passed and overall not in passed:
                    failures.append(f'{label} validation overall status is not passed')

    payload_path_field = contract.get('payload_path_field')
    payload_ref = get_field(record, payload_path_field) if payload_path_field else None
    rows = load_payload(
        resolve(payload_ref) if payload_ref else pathlib.Path(),
        contract.get('payload_format') or 'json',
    ) if payload_path_field else []
    if payload_path_field and not rows:
        failures.append(f'{label} payload has no records')
    for row_index, row in enumerate(rows):
        if not isinstance(row, dict):
            failures.append(f'{label} payload row {row_index} is not an object')
            continue
        for field in contract.get('required_fields', []):
            if get_field(row, field) is None:
                failures.append(f'{label} payload row {row_index} lacks {field}')
    for field in contract.get('non_constant_fields', []):
        values = {
            json.dumps(get_field(row, field), sort_keys=True, separators=(',', ':'))
            for row in rows if isinstance(row, dict)
        }
        if len(values) <= 1:
            failures.append(f'{label} payload field is constant or absent: {field}')

    request_path_field = contract.get('request_path_field')
    requested_count_field = contract.get('requested_count_field')
    if request_path_field and requested_count_field:
        request_ref = get_field(record, request_path_field)
        request = load_json(resolve(request_ref), f'{label} request') if request_ref else None
        requested = get_field(request, requested_count_field) if request else None
        try:
            requested_value = int(requested)
            if requested_value <= 0:
                failures.append(f'{label} requested count is not positive: {requested}')
            elif len(rows) < requested_value:
                failures.append(
                    f'{label} delivered count {len(rows)} is below requested count {requested}'
                )
        except (TypeError, ValueError):
            failures.append(f'{label} requested count is not numeric')

    response_path_field = contract.get('response_path_field')
    if response_path_field:
        response_ref = get_field(record, response_path_field)
        if not response_ref:
            failures.append(f'{label} registry has no response path')
        response = load_json(resolve(response_ref), f'{label} response') if response_ref else None
        if isinstance(response, dict):
            for cell_field, response_field in contract.get('response_identity_fields', {}).items():
                if normalized(get_field(cell, cell_field)) != normalized(
                    get_field(response, response_field)
                ):
                    failures.append(
                        f'{label} response identity mismatch: {cell_field}!={response_field}'
                    )

    signature_fields = contract.get('series_value_fields', [])
    if rows and signature_fields:
        tokens = tuple(
            tuple(
                json.dumps(get_field(row, field), sort_keys=True, separators=(',', ':'))
                for field in signature_fields
            )
            for row in rows if isinstance(row, dict)
        )
        digest = hashlib.sha256(
            json.dumps(tokens, separators=(',', ':')).encode()
        ).hexdigest()
        series.append((identity, digest, tokens))
        field_steps = []
        for field in signature_fields:
            try:
                values = [float(get_field(row, field)) for row in rows if isinstance(row, dict)]
            except (TypeError, ValueError):
                continue
            if len(values) < 3:
                continue
            steps = tuple(round(values[index] - values[index - 1], 12) for index in range(1, len(values)))
            if steps and len(set(steps)) == 1:
                failures.append(f'{label} payload field has a constant first difference: {field}')
            field_steps.append((field, steps))
        if field_steps:
            step_series.append((identity, tuple(field_steps)))

overlap_rows = int(contract.get('series_overlap_min_rows') or 0)
for index, (identity, digest, tokens) in enumerate(series):
    for other_identity, other_digest, other_tokens in series[:index]:
        if digest == other_digest:
            failures.append(
                f'distinct cells {other_identity} and {identity} have identical payload series'
            )
            continue
        if overlap_rows > 0 and len(tokens) >= overlap_rows and len(other_tokens) >= overlap_rows:
            windows = {
                tokens[offset:offset + overlap_rows]
                for offset in range(len(tokens) - overlap_rows + 1)
            }
            other_windows = {
                other_tokens[offset:offset + overlap_rows]
                for offset in range(len(other_tokens) - overlap_rows + 1)
            }
            if windows.intersection(other_windows):
                failures.append(
                    f'distinct cells {other_identity} and {identity} share a declared payload-series window'
                )

for index, (identity, steps) in enumerate(step_series):
    for other_identity, other_steps in step_series[:index]:
        if steps == other_steps:
            failures.append(
                f'distinct cells {other_identity} and {identity} have identical declared field-step series'
            )

if failures:
    print(json.dumps({'status': 'failed', 'failure_count': len(failures), 'failures': failures}, indent=2))
    raise SystemExit(1)
print(json.dumps({
    'status': 'substantive_deliverable_verified',
    'required_cells': len(required),
    'verified_cells': len(indexed),
    'series_checked': len(series),
}, indent=2))
PY"#;
